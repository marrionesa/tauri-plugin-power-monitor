//! Windows backend.
//!
//! Queries use [`GetSystemPowerStatus`]. Events arrive as
//! [`WM_POWERBROADCAST`] messages on a hidden top-level window owned by a
//! dedicated thread (message-only windows do **not** receive broadcast
//! messages, so a never-shown `WS_POPUP` window is required — the same
//! approach used by Chromium's `base::PowerMonitor`).
//!
//! `RegisterSuspendResumeNotification` is required to receive
//! [`PBT_APMSUSPEND`] on machines with Modern Standby, and
//! `RegisterPowerSettingNotification` provides granular battery percentage
//! and AC/DC change notifications.

use std::sync::Mutex;
use std::thread::JoinHandle;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    GetSystemPowerStatus, RegisterPowerSettingNotification, RegisterSuspendResumeNotification,
    UnregisterPowerSettingNotification, UnregisterSuspendResumeNotification, HPOWERNOTIFY,
    SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, PostThreadMessageW, RegisterClassExW, SetWindowLongPtrW, TranslateMessage,
    UnregisterClassW, DEVICE_NOTIFY_WINDOW_HANDLE, GWLP_USERDATA, MSG, PBT_APMPOWERSTATUSCHANGE,
    PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, WM_POWERBROADCAST, WM_QUIT,
    WNDCLASSEXW, WNDCLASS_STYLES, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::{state_from_system_power_status, PowerBackend};
use crate::error::{Error, Result};
use crate::events::NativeEvent;
use crate::events::NativeEventSender;
use crate::models::PowerState;

/// State shared between the listener thread and the window procedure.
struct ThreadContext {
    events: NativeEventSender,
}

/// The running Windows event listener.
struct WindowsListener {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

/// Windows backend handle.
pub(crate) struct WindowsPowerBackend {
    listener: Mutex<Option<WindowsListener>>,
}

impl WindowsPowerBackend {
    pub(crate) fn new() -> Self {
        Self {
            listener: Mutex::new(None),
        }
    }
}

impl Default for WindowsPowerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerBackend for WindowsPowerBackend {
    fn state(&self) -> Result<PowerState> {
        // SAFETY: `status` is a plain, fully-owned stack struct and the
        // function only writes to it. The call is safe from any thread.
        let mut status = SYSTEM_POWER_STATUS::default();
        unsafe {
            GetSystemPowerStatus(&mut status).map_err(Error::query)?;
        }
        Ok(state_from_system_power_status(
            status.ACLineStatus,
            status.BatteryFlag,
            status.BatteryLifePercent,
            status.BatteryLifeTime,
            status.BatteryFullLifeTime,
        ))
    }

    fn start(&self, events: NativeEventSender) -> Result<()> {
        let mut listener = self.listener.lock().unwrap_or_else(|e| e.into_inner());
        if listener.is_some() {
            return Ok(());
        }

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<u32, String>>();

        let thread = std::thread::Builder::new()
            .name("tauri-plugin-power-monitor".into())
            .spawn(move || {
                let result = run_message_loop(events, ready_tx);
                if let Err(e) = result {
                    log::warn!("windows power event listener stopped: {e}");
                }
            })
            .map_err(Error::listener)?;

        match ready_rx.recv() {
            Ok(Ok(thread_id)) => {
                *listener = Some(WindowsListener {
                    thread_id,
                    thread: Some(thread),
                });
                Ok(())
            }
            // The thread reported a setup failure (or died): join it before
            // surfacing the error so no thread is left behind.
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(Error::listener(e))
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::listener(
                    "the power event listener thread exited unexpectedly",
                ))
            }
        }
    }

    fn stop(&self) {
        let mut listener = self.listener.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut listener) = listener.take() {
            // SAFETY: `thread_id` was captured on the listener thread after
            // its message queue was created, which is the requirement for
            // `PostThreadMessageW`. WM_QUIT makes `GetMessageW` return 0,
            // ending the loop; the thread then cleans up its own resources.
            unsafe {
                let _ = PostThreadMessageW(listener.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            if let Some(thread) = listener.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Creates the hidden window, registers every notification, and runs the
/// message loop until `WM_QUIT` is posted. All resources created here are
/// destroyed here, on the same thread.
fn run_message_loop(
    events: NativeEventSender,
    ready: std::sync::mpsc::Sender<std::result::Result<u32, String>>,
) -> std::result::Result<(), String> {
    // SAFETY: this entire function runs on a single dedicated thread and only
    // touches thread-owned resources. The context pointer installed as window
    // user data is freed at the end of this function, after `DestroyWindow`
    // and after the class is unregistered, so the window procedure can no
    // longer receive calls that would dereference it.
    unsafe {
        let context = Box::into_raw(Box::new(ThreadContext { events }));

        // Class names are process-global: append the process id to avoid
        // colliding with another library using the same name.
        let class_name: Vec<u16> = format!("TauriPowerMonitorWnd_{}", std::process::id())
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let class_name_ptr = PCWSTR(class_name.as_ptr());

        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let instance = windows::Win32::Foundation::HINSTANCE::from(module);
        let window_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(power_wnd_proc),
            hInstance: instance,
            lpszClassName: class_name_ptr,
            ..Default::default()
        };

        if RegisterClassExW(&window_class) == 0 {
            drop(Box::from_raw(context));
            return Err("RegisterClassExW failed".into());
        }

        // A hidden top-level window: message-only windows (HWND_MESSAGE) do
        // not receive broadcast messages such as WM_POWERBROADCAST, so the
        // window must be a real (never shown) top-level window.
        let window = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class_name_ptr,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
        .map_err(|e| {
            let _ = UnregisterClassW(class_name_ptr, Some(instance));
            drop(Box::from_raw(context));
            e.to_string()
        })?;

        SetWindowLongPtrW(window, GWLP_USERDATA, context as isize);

        // On machines with Modern Standby, PBT_APMSUSPEND is only delivered
        // to windows registered through this API (same requirement as
        // Chromium).
        let suspend_notification: Option<HPOWERNOTIFY> =
            RegisterSuspendResumeNotification(window.into(), DEVICE_NOTIFY_WINDOW_HANDLE).ok();

        // Granular battery percentage and AC/DC change notifications
        // (PBT_POWERSETTINGCHANGE).
        let power_notifications: Vec<HPOWERNOTIFY> = [
            RegisterPowerSettingNotification(
                window.into(),
                &GUID_BATTERY_PERCENTAGE_REMAINING,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
            .ok(),
            RegisterPowerSettingNotification(
                window.into(),
                &GUID_ACDC_POWER_SOURCE,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
            .ok(),
        ]
        .into_iter()
        .flatten()
        .collect();

        let _ = ready.send(Ok(GetCurrentThreadId()));

        let mut message = MSG::default();
        loop {
            let ret = GetMessageW(&mut message, None, 0, 0);
            if ret.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&message);
            let _ = DispatchMessageW(&message);
        }

        // Cleanup, in reverse order of creation, on the owning thread.
        for notification in power_notifications {
            let _ = UnregisterPowerSettingNotification(notification);
        }
        if let Some(notification) = suspend_notification {
            let _ = UnregisterSuspendResumeNotification(notification);
        }
        let _ = DestroyWindow(window);
        let _ = UnregisterClassW(class_name_ptr, Some(instance));
        drop(Box::from_raw(context));
        Ok(())
    }
}

/// Window procedure handling `WM_POWERBROADCAST`.
///
/// SAFETY: `hwnd` must belong to the thread that created it (guaranteed by
/// construction). The user data pointer is only dereferenced after it was set
/// by `SetWindowLongPtrW` and before the owning thread frees the context.
unsafe extern "system" fn power_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_POWERBROADCAST {
        let context = match GetWindowLongPtrW(hwnd, GWLP_USERDATA) {
            0 => return DefWindowProcW(hwnd, message, wparam, lparam),
            pointer => &*(pointer as *const ThreadContext),
        };
        // SAFETY: the context outlives the window (freed after DestroyWindow
        // on the owning thread) and `Sender::send` is thread-safe.
        match wparam.0 as u32 {
            PBT_APMSUSPEND => {
                let _ = context.events.send(NativeEvent::Suspend);
            }
            // PBT_APMRESUMEAUTOMATIC is always sent on resume; if
            // PBT_APMRESUMESUSPEND follows, it is caused by user input and
            // would be a duplicate (same rule as Chromium).
            PBT_APMRESUMEAUTOMATIC => {
                let _ = context.events.send(NativeEvent::Resume);
            }
            PBT_APMPOWERSTATUSCHANGE | PBT_POWERSETTINGCHANGE => {
                let _ = context.events.send(NativeEvent::Requery);
            }
            _ => {}
        }
        return LRESULT(1);
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}
