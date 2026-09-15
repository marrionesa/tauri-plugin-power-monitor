# Contributing

Thank you for contributing to `tauri-plugin-power-monitor`, a community Tauri 2 desktop plugin.

## Requirements

- Rust 1.88 or newer, as specified by the MSRV in `Cargo.toml`
- Node.js and pnpm
- Tauri CLI (`cargo install tauri-cli` or the project-standard installation)
- Linux development packages required by Tauri, including GTK and WebKitGTK

Clone the repository, then install JavaScript dependencies with `pnpm install`.

## Checks

Run the Rust checks:

```bash
cargo fmt --all --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Build and test the guest API:

```bash
pnpm build
pnpm typecheck
pnpm test
```

The committed `dist-js/` files and `api-iife.js` must stay synchronized with `guest-js/`. Do not edit generated output by hand when the Rollup build can regenerate it.

## Example

```bash
cd examples/api
npm install
npm run build
cargo tauri dev
```

The example is a static esbuild frontend. It does not use a development server.

## Changes and tests

Keep the plugin read-only and event-driven. Do not introduce polling, power control, telemetry, or unrelated platform features. Add or update focused tests for behavior changes. Platform-specific code belongs in the corresponding file under `src/platform/`; preserve the native Windows, macOS, and Linux APIs and document platform limitations honestly.

Open a focused pull request describing the behavior change, platform impact, and validation performed.
