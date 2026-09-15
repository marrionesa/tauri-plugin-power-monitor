//! macOS backend.
//!
//! Queries use the IOPowerSources API (`IOPSCopyPowerSourcesInfo` →
//! `IOPSCopyPowerSourcesList` → `IOPSGetPowerSourceDescription`).
//!
//! Events come from two IOKit sources, both serviced by a dedicated run loop
//! thread that exists solely for this plugin:
//!
//! - `IOPSNotificationCreateRunLoopSource` — fires on power source changes
//!   (AC plug/unplug, battery percentage, time estimates).
//! - `IORegisterForSystemPower` (the Apple QA1340 pattern) — system sleep/wake
//!   notifications. `IOAllowPowerChange` **must** be acknowledged for
//!   `kIOMessageCanSystemSleep` and `kIOMessageSystemWillSleep`, otherwise the
//!   system delays sleep by ~30 seconds.
//!
//! The IOKit user-space C API has no maintained Rust binding, so the small
//! surface needed here is declared manually below. CoreFoundation types come
//! from the `core-foundation` crate.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopRef, CFRunLoopSource, CFRunLoopStop,
};
use core_foundation::string::CFString;

use super::{
    percentage_from_iops_capacities, power_source_from_iops_state, seconds_from_iops_minutes,
    PowerBackend,
};
use crate::error::{Error, Result};
use crate::events::{NativeEvent, NativeEventSender};
use crate::models::{BatteryInfo, PowerSource, PowerState};

// ---------------------------------------------------------------------------
// IOKit externs (no maintained Rust binding crate covers these).
// ---------------------------------------------------------------------------

/// `io_object_t` / `io_service_t` / `mach_port_t` — `natural_t` (unsigned int).
type IoObject = u32;
/// `IONotificationPortRef` — opaque pointer.
type IoNotificationPortRef = *mut c_void;
/// `io_user_reference_t` (64-bit).
type IoUserReference = u64;
/// `kern_return_t`.
type KernReturn = i32;

/// `IOPowerSourceCallbackType`.
type IopsCallback = Option<unsafe extern "C" fn(context: *mut c_void)>;
/// `IOServiceInterestCallback`.
type IoServiceInterestCallback =
    Option<unsafe extern "C" fn(*mut c_void, IoObject, u32, *mut c_void)>;

/// `kIOMessageCanSystemSleep` = `iokit_common_msg(0x270)` where
/// `iokit_common_msg(x) = sys_iokit | sub_iokit_common | x` and
/// `sys_iokit = err_system(0x38) = 0xE0000000` (xnu's `IOKit/IOMessage.h`).
const K_IO_MESSAGE_CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
/// `kIOMessageSystemWillSleep` = `iokit_common_msg(0x280)`.
const K_IO_MESSAGE_SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;
/// `kIOMessageSystemHasPoweredOn` = `iokit_common_msg(0x300)`.
const K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON: u32 = 0xE000_0300;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> *const c_void;
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, ps: CFTypeRef) -> *const c_void;
    fn IOPSRelease(blob: CFTypeRef);
    fn IOPSNotificationCreateRunLoopSource(
        callback: IopsCallback,
        context: *mut c_void,
    ) -> *const c_void;

    fn IORegisterForSystemPower(
        refcon: *mut c_void,
        the_port_ref: *mut IoNotificationPortRef,
        callback: IoServiceInterestCallback,
        notifier: *mut IoUserReference,
    ) -> IoObject;
    fn IONotificationPortGetRunLoopSource(notify_port: IoNotificationPortRef) -> *const c_void;
    fn IONotificationPortDestroy(notify_port: IoNotificationPortRef);
    fn IODeregisterForSystemPower(notifier: *mut IoUserReference) -> KernReturn;
    fn IOAllowPowerChange(power_ref: IoObject, id: isize);
    fn IOServiceClose(service: IoObject) -> KernReturn;
}

// ---------------------------------------------------------------------------
// Backend.
// ---------------------------------------------------------------------------

/// State shared with the C callbacks. Only ever dereferenced from the run
/// loop thread; `root_port` is published before the loop starts running.
struct MacosContext {
    events: NativeEventSender,
    root_port: AtomicU32,
}

/// `CFRunLoopRef` wrapper allowed to cross threads: `CFRunLoopStop` is
/// thread-safe and the run loop itself is run and released by the listener
/// thread, which is joined before this plugin considers itself stopped.
#[repr(transparent)]
struct SendRunLoopRef(CFRunLoopRef);
// SAFETY: see type documentation.
unsafe impl Send for SendRunLoopRef {}

/// The running macOS event listener.
struct MacosListener {
    runloop: SendRunLoopRef,
    thread: Option<JoinHandle<()>>,
}

/// macOS backend handle.
pub(crate) struct MacosPowerBackend {
    listener: Mutex<Option<MacosListener>>,
    /// Serializes IOPS queries: the thread-safety of the snapshot API is not
    /// explicitly documented, so queries (dispatcher thread + command calls)
    /// are serialized to stay on the safe side.
    query_lock: Mutex<()>,
}

impl MacosPowerBackend {
    pub(crate) fn new() -> Self {
        Self {
            listener: Mutex::new(None),
            query_lock: Mutex::new(()),
        }
    }
}

impl Default for MacosPowerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerBackend for MacosPowerBackend {
    fn state(&self) -> Result<PowerState> {
        let _guard = self.query_lock.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the IOPS snapshot API is called with valid, owned arguments
        // and the returned blob is released on every exit path.
        unsafe {
            let blob = IOPSCopyPowerSourcesInfo();
            if blob.is_null() {
                return Err(Error::query(
                    "IOPSCopyPowerSourcesInfo returned a null snapshot",
                ));
            }
            let state = state_from_power_sources(blob);
            IOPSRelease(blob);
            state
        }
    }

    fn start(&self, events: NativeEventSender) -> Result<()> {
        let mut listener = self.listener.lock().unwrap_or_else(|e| e.into_inner());
        if listener.is_some() {
            return Ok(());
        }

        let (ready_tx, ready_rx) = channel::<std::result::Result<SendRunLoopRef, String>>();

        let thread = std::thread::Builder::new()
            .name("tauri-plugin-power-monitor".into())
            .spawn(move || run_listener(events, &ready_tx))
            .map_err(Error::listener)?;

        match ready_rx.recv() {
            Ok(Ok(runloop)) => {
                *listener = Some(MacosListener {
                    runloop,
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
            // SAFETY: CFRunLoopStop is thread-safe; the listener thread owns
            // the loop and cleans up all resources once it exits.
            unsafe {
                CFRunLoopStop(listener.runloop.0);
            }
            if let Some(thread) = listener.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Runs the listener on the current thread until [`CFRunLoopStop`] is called
/// on its run loop. Signals `ready` with the run loop reference (or the setup
/// error) before servicing the loop.
fn run_listener(
    events: NativeEventSender,
    ready: &Sender<std::result::Result<SendRunLoopRef, String>>,
) {
    // Context handed to the C callbacks; freed at the end of this function,
    // after the loop has stopped and the sources were removed, so no callback
    // can outlive it.
    let context = Arc::into_raw(Arc::new(MacosContext {
        events,
        root_port: AtomicU32::new(0),
    })) as *mut c_void;

    // SAFETY: all IOKit/CoreFoundation calls below run on this dedicated
    // thread and every value created here is destroyed on the same thread.
    unsafe {
        let runloop = CFRunLoop::get_current();

        // Power source changes (AC plug/unplug, battery, estimates).
        let iops_source_raw =
            IOPSNotificationCreateRunLoopSource(Some(iops_notification_callback), context);
        if iops_source_raw.is_null() {
            drop(Arc::from_raw(context as *const MacosContext));
            let _ = ready.send(Err("IOPSNotificationCreateRunLoopSource failed".into()));
            return;
        }
        // Create rule: released when the wrapper is dropped.
        let iops_source: CFRunLoopSource =
            CFRunLoopSource::wrap_under_create_rule(iops_source_raw as _);
        runloop.add_source(&iops_source, kCFRunLoopDefaultMode);

        // System sleep/wake notifications.
        let mut notify_port: IoNotificationPortRef = std::ptr::null_mut();
        let mut notifier: IoUserReference = 0;
        let root_port = IORegisterForSystemPower(
            context,
            &mut notify_port,
            Some(system_power_callback),
            &mut notifier,
        );
        if root_port == 0 || notify_port.is_null() {
            runloop.remove_source(&iops_source, kCFRunLoopDefaultMode);
            drop(iops_source);
            drop(Arc::from_raw(context as *const MacosContext));
            let _ = ready.send(Err("IORegisterForSystemPower failed".into()));
            return;
        }

        // Get rule: this run loop source is owned by the notification port
        // and is destroyed by IONotificationPortDestroy below.
        let power_source: CFRunLoopSource = CFRunLoopSource::wrap_under_get_rule(
            IONotificationPortGetRunLoopSource(notify_port) as _,
        );
        runloop.add_source(&power_source, kCFRunLoopDefaultMode);

        // The callbacks can only fire once this thread services the loop
        // below, so publishing the port happens-before any callback.
        (*(context as *const MacosContext))
            .root_port
            .store(root_port, Ordering::Release);

        let send_ref = SendRunLoopRef(runloop.as_CFTypeRef() as CFRunLoopRef);
        let _ = ready.send(Ok(send_ref));

        CFRunLoop::run_current();

        // Cleanup in reverse order of creation, mirroring Chromium's
        // `PowerMonitorDeviceSource::PlatformDestroy`.
        runloop.remove_source(&power_source, kCFRunLoopDefaultMode);
        runloop.remove_source(&iops_source, kCFRunLoopDefaultMode);
        drop(iops_source);

        IODeregisterForSystemPower(&mut notifier);
        IONotificationPortDestroy(notify_port);
        IOServiceClose(root_port);
        drop(Arc::from_raw(context as *const MacosContext));
    }
}

/// IOPS notification callback: any power source change triggers a requery.
///
/// SAFETY: `context` originates from `Arc::into_raw` in `run_listener` and is
/// valid for the whole lifetime of the run loop sources.
unsafe extern "C" fn iops_notification_callback(context: *mut c_void) {
    let context = &*(context as *const MacosContext);
    let _ = context.events.send(NativeEvent::Requery);
}

/// System power callback (sleep/wake), following Apple QA1340.
///
/// SAFETY: `refcon` originates from `Arc::into_raw` in `run_listener`; the
/// message argument is the notification id to acknowledge.
unsafe extern "C" fn system_power_callback(
    refcon: *mut c_void,
    _service: IoObject,
    message_type: u32,
    message_argument: *mut c_void,
) {
    let context = &*(refcon as *const MacosContext);
    match message_type {
        K_IO_MESSAGE_SYSTEM_WILL_SLEEP => {
            let _ = context.events.send(NativeEvent::Suspend);
            // Acknowledge, otherwise the system delays sleep by ~30 seconds.
            IOAllowPowerChange(
                context.root_port.load(Ordering::Acquire),
                message_argument as isize,
            );
        }
        K_IO_MESSAGE_CAN_SYSTEM_SLEEP => {
            // Idle sleep request: we cannot veto it (we are a monitor), so
            // acknowledge immediately.
            IOAllowPowerChange(
                context.root_port.load(Ordering::Acquire),
                message_argument as isize,
            );
        }
        // "System and its devices have woken up" — the right point for an
        // application-facing resume event.
        K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON => {
            let _ = context.events.send(NativeEvent::Resume);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// IOPS dictionary reading.
// ---------------------------------------------------------------------------

/// Reads the current power state from an IOPS snapshot blob.
///
/// SAFETY: `blob` must be a valid value returned by `IOPSCopyPowerSourcesInfo`
/// that has not been released yet.
unsafe fn state_from_power_sources(blob: CFTypeRef) -> Result<PowerState> {
    let list_raw = IOPSCopyPowerSourcesList(blob);
    if list_raw.is_null() {
        return Err(Error::query("IOPSCopyPowerSourcesList returned null"));
    }
    // Create rule: the array is released when dropped.
    let list: CFArray<CFType> = CFArray::wrap_under_create_rule(list_raw as _);

    let mut power_source = PowerSource::Unknown;
    let mut battery: Option<BatteryInfo> = None;

    for entry in list.get_all_values() {
        let description_raw = IOPSGetPowerSourceDescription(blob, entry);
        if description_raw.is_null() {
            continue;
        }
        // Get rule: the dictionary belongs to the snapshot blob.
        let description: CFDictionary<CFString, CFType> =
            CFDictionary::wrap_under_get_rule(description_raw as _);

        let is_present = read_bool(&description, "Is Present").unwrap_or(false);
        if !is_present {
            continue;
        }

        let state_string = read_string(&description, "Power Source State");

        let info = BatteryInfo {
            is_present: true,
            percentage: percentage_from_iops_capacities(
                read_i64(&description, "Current Capacity"),
                read_i64(&description, "Max Capacity"),
            ),
            is_charging: read_bool(&description, "Is Charging"),
            time_to_empty: read_i64(&description, "Time to Empty")
                .and_then(seconds_from_iops_minutes),
            time_to_full: read_i64(&description, "Time to Full Charge")
                .and_then(seconds_from_iops_minutes),
        };

        if battery.is_none() {
            // Laptops expose a single aggregate internal battery; should
            // several present power sources be reported, the first one wins.
            battery = Some(info);
            power_source = state_string
                .as_deref()
                .and_then(power_source_from_iops_state)
                .unwrap_or(PowerSource::Unknown);
        }
    }

    // A Mac without any present battery is, by definition, running on
    // external power (mirrors ACLineStatus = 1 on desktop Windows machines).
    if battery.is_none() {
        power_source = PowerSource::Ac;
    }

    Ok(PowerState {
        power_source,
        battery,
    })
}

fn read_string(dictionary: &CFDictionary<CFString, CFType>, key: &'static str) -> Option<String> {
    let value = dictionary.find(CFString::from_static_string(key))?;
    value.downcast::<CFString>().map(|s| s.to_string())
}

fn read_bool(dictionary: &CFDictionary<CFString, CFType>, key: &'static str) -> Option<bool> {
    let value = dictionary.find(CFString::from_static_string(key))?;
    value.downcast::<CFBoolean>().map(bool::from)
}

fn read_i64(dictionary: &CFDictionary<CFString, CFType>, key: &'static str) -> Option<i64> {
    let value = dictionary.find(CFString::from_static_string(key))?;
    value.downcast::<CFNumber>().and_then(|n| n.to_i64())
}
