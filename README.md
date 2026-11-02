# RusTor

A Rust-powered desktop BitTorrent application in development, with a React and TypeScript interface planned through Tauri 2.

Status: native engine foundation implemented and tested on Windows. The desktop interface and search providers are not implemented yet; there is no desktop release.

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

This is a library foundation, with no end-user CLI or TUI. The next product step is the Tauri bridge and desktop download flow. Searchable-content coverage will be added through provider integrations; the torrent library does not supply a catalog.

## Acknowledgements

RusTor is an independent project inspired by [Torlink](https://github.com/baairon/torlink), developed by bairon.dev and its contributors. Its search, torrent-management, and persistence behavior provides the foundation for our planned Rust port. RusTor is not an official Torlink release and does not develop or ship its TUI.

Torlink is licensed under the MIT License, copyright 2026 bairon.dev. RusTor's input normalization and lifecycle behavior are informed by its magnet, queue, and persistence implementations. No Torlink source is vendored here. As source is directly adapted, this section will identify those portions and applicable notices will be preserved.

The native protocol implementation is provided by [librqbit / rqbit](https://github.com/ikatson/rqbit), developed by Igor Katson and contributors and licensed under Apache-2.0. Our local transfer tests draw on its public API examples and testing approach. RusTor does not claim authorship of that protocol implementation.

RusTor's original code is licensed under the [MIT License](LICENSE). Third-party dependencies retain their own licenses; release packaging must include their applicable notices.
