# RusTor

A Rust-powered desktop BitTorrent application in development, with a React and TypeScript interface delivered through Tauri 2.

Status: the native engine and first desktop Downloads screen are implemented. Local transfer and service tests run on Windows. Search providers, advanced queue management, and release installers are not implemented yet.

Frontend: React, TypeScript, Bun, Vite, and Tailwind CSS v4. The backend will port Torlink's useful non-TUI behavior into Rust, using a proven torrent protocol library where suitable.

## Engine foundation

The `rustor-torrent` crate integrates librqbit 9.1.0 in-process, without a Node.js or Bun runtime. It supports validated magnets, hex/base32 v1 hashes, and `.torrent` metadata bytes; destination selection; concurrent transfers; progress snapshots; pause/resume; and engine-owned session restoration. Initialization and per-torrent peer limits are configurable. A session-directory lock prevents concurrent processes from writing the same engine state.

The adapter reserves numeric session IDs atomically so simultaneous additions remain distinct with JSON persistence. This behavior is guarded by a real two-torrent transfer test.

Tests use generated data and local peers. They cover byte integrity, duplicate and simultaneous additions, active pause/resume, paused-state restoration across process restarts, and recovery of completed torrents after forced process termination. Partial-transfer crash recovery, disk-full handling, public-network interoperability, and macOS/Linux behavior still need validation. v2-only torrents are currently rejected.

With the Rust toolchain and platform C/C++ build prerequisites installed:

```sh
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

## Desktop development

The dark Downloads screen connects to the real Rust session through Tauri commands and live snapshots. It includes magnet/infohash entry, native `.torrent` and destination pickers, transfer filters, progress and speed reporting, details, and pause/resume controls. File reads stay in Rust and use handles returned by the native picker; the frontend cannot submit an arbitrary file path. There is no end-user CLI or TUI.

Install Bun 1.3.5, a current Node.js development runtime compatible with Vite, Rust, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). Windows needs the Microsoft C++ build tools and WebView2. Node.js and Bun are development tools and are not bundled as application runtimes.

```sh
bun install --frozen-lockfile
bun run desktop:dev
```

Checks and standalone desktop build:

```sh
bun run check
bun run test
bun run build
bun run desktop:build --debug
```

The desktop build embeds the frontend and produces an executable without an installer. Omitting `--debug` builds in release mode. Opening the Vite URL in a browser shows an explicitly labeled layout preview; actual downloads require the desktop application. The engine uses the OS application-data directory and defaults to `Downloads/RusTor` for payloads. Closing the app stops the engine; it does not continue downloading in a tray process.

Validation so far includes the native window rendering with an engine-ready response, frontend behavior tests, and a desktop-service integration test that selects handles, downloads generated data from a loopback peer, verifies bytes, and pauses/resumes. A complete native picker-to-download UI automation pass and macOS/Linux validation remain outstanding. GitHub Actions runs the Windows checks and debug executable build.

Searchable-content coverage will be added through provider integrations; the torrent library does not supply a catalog. Drag-and-drop, notifications, light theme, deletion, retry controls, file selection, and download queue policies are follow-up work.

## Acknowledgements

RusTor is an independent project inspired by [Torlink](https://github.com/baairon/torlink), developed by bairon.dev and its contributors. Its search, torrent-management, and persistence behavior provides the foundation for our planned Rust port. RusTor is not an official Torlink release and does not develop or ship its TUI.

Torlink is licensed under the MIT License, copyright 2026 bairon.dev. RusTor's input normalization and lifecycle behavior are informed by its magnet, queue, and persistence implementations. No Torlink source is vendored here. As source is directly adapted, this section will identify those portions and applicable notices will be preserved.

The native protocol implementation is provided by [librqbit / rqbit](https://github.com/ikatson/rqbit), developed by Igor Katson and contributors and licensed under Apache-2.0. Our local transfer tests draw on its public API examples and testing approach. RusTor does not claim authorship of that protocol implementation.

RusTor's original code is licensed under the [MIT License](LICENSE). Third-party dependencies retain their own licenses; release packaging must include their applicable notices.
