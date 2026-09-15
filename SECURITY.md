# Security

`tauri-plugin-power-monitor` is a read-only desktop plugin. It observes battery and power state, but does not change power policies or control shutdown, reboot, hibernate, sleep, or wake behavior. It does not collect data or send telemetry, and it does not execute shell commands to obtain power information.

On Linux, the plugin uses UPower and zbus for event-driven queries and notifications, with a sysfs query fallback when UPower or D-Bus is unavailable. Suspend and resume events require systemd-logind.

Please report vulnerabilities responsibly. When GitHub Security Advisories are enabled for the repository, use a private advisory. Otherwise, contact the maintainer through the private reporting mechanism defined by the repository. Do not disclose an unpatched vulnerability publicly.
