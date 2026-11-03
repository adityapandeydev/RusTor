# RusTor

A Rust-powered desktop BitTorrent application in development, with a React and TypeScript interface delivered through Tauri 2.

Status: the native engine and first desktop Downloads screen are implemented. Local transfer and service tests run on Windows. Search providers, advanced queue management, and release installers are not implemented yet.

Frontend: React, TypeScript, Bun, Vite, and Tailwind CSS v4. The backend will port Torlink's useful non-TUI behavior into Rust, using a proven torrent protocol library where suitable.

## Engine foundation

The `rustor-torrent` crate integrates librqbit 9.1.0 in-process, without a Node.js or Bun runtime. It supports validated magnets, hex/base32 v1 hashes, and `.torrent` metadata bytes; destination selection; concurrent transfers; metadata-only file previews and selective downloads; progress snapshots; pause/resume and removal with files kept; and engine-owned session restoration. Initialization and per-torrent peer limits are configurable. New transfers retain their exclusively created filesystem handles for in-process retry; stale storage wrappers are invalidated before recovery. A session-directory lock prevents concurrent processes from writing the same engine state.

The adapter reserves numeric session IDs atomically so simultaneous additions remain distinct with JSON persistence. This behavior is guarded by a real two-torrent transfer test.

Tests use generated data and local peers. They cover byte integrity, duplicate and simultaneous additions, active pause/resume, paused-state restoration across process restarts, and recovery of completed torrents after forced process termination. Partial-transfer crash recovery, disk-full handling, public-network interoperability, and macOS/Linux behavior still need validation. v2-only torrents are currently rejected.

With the Rust toolchain and platform C/C++ build prerequisites installed:

```sh
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

## Desktop development

The Downloads screen connects to the real Rust session through Tauri commands and live snapshots. It includes magnet/infohash entry, native `.torrent` and destination pickers, transfer filters, progress and speed reporting, details, and pause/resume, stop/remove, and failed-transfer retry controls. Failed transfers show recovery guidance and expandable engine error details without opening the row. Retry rechecks existing data in place and retains the destination and file selection. The × action stops downloading or seeding and removes the torrent from the saved session while keeping complete and partial files on disk. Dark mode uses navy surfaces with peach accents; a matching warm light mode is available from the header toggle, and the appearance preference is saved locally. The add flow loads metadata first, then lets you choose files and review their total size before starting. Large lists support search and pagination. Selections survive session restoration; skipped files can still receive data from shared torrent pieces. New previews currently support up to 10,000 files. File reads stay in Rust and use handles returned by the native picker; the frontend cannot submit an arbitrary file path. There is no end-user CLI or TUI.

The header's Download settings control saves a default download folder and session-wide download/upload limits. Limits use KiB/s (1,024 bytes/s), with 0 for unlimited and finite values from 16 to 976,562 KiB/s. They apply to active and future transfers; the limiter permits bursts and governs torrent payload data. Changing the default folder affects new torrents only, preserves native torrent subfolders, and leaves existing files in place. A per-torrent folder choice still overrides the default.

Preferences use a versioned, size-bounded `settings.json` in the application-data engine folder. Saves replace the file atomically after flushing the temporary file. A failed save leaves active preferences unchanged. Malformed or unsupported saved settings are preserved and reported at startup rather than silently reset; repair or remove that file while the app is closed to recover. Power-loss durability and network-drive behavior have not been validated.

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

Validation so far includes the native window rendering with an engine-ready response, frontend behavior tests, and a desktop-service integration test that selects handles, downloads generated data from a loopback peer, verifies bytes, and pauses/resumes/removes. Native file selection, resizing, and session persistence have also been checked manually on Windows. A complete native picker-to-download UI automation pass and macOS/Linux validation remain outstanding. GitHub Actions runs the Windows checks and debug executable build.

Searchable-content coverage will be added through provider integrations; the torrent library does not supply a catalog. Drag-and-drop, notifications, payload deletion, changing file selection after addition, and download queue policies are follow-up work.

## Acknowledgements

RusTor is an independent project inspired by [Torlink](https://github.com/baairon/torlink), developed by bairon.dev and its contributors. Its search, torrent-management, and persistence behavior provides the foundation for our planned Rust port. RusTor is not an official Torlink release and does not develop or ship its TUI.

Torlink is licensed under the MIT License, copyright 2026 bairon.dev. RusTor's input normalization and lifecycle behavior are informed by its magnet, queue, and persistence implementations. The Rust search contracts, tracker-preserving duplicate merge, ordering, and token matching in `crates/rustor-search` port behavior from Torlink's [source types](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/types.ts), [deduplication](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/ui/dedupe.ts), [sorting](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/ui/sort.ts), [filtering](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/ui/filter.ts), and [magnet helpers](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/magnet.ts) at revision `df059311d0898b7245dd842153c1ff7c892275ee`. Its TypeScript source and terminal UI are not distributed. The applicable upstream notice is retained here:

<details>
<summary>Torlink MIT License notice</summary>

```text
MIT License

Copyright (c) 2026 bairon.dev

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

</details>

The native protocol implementation is provided by [librqbit / rqbit](https://github.com/ikatson/rqbit), developed by Igor Katson and contributors and licensed under Apache-2.0. Our local transfer tests draw on its public API examples and testing approach. RusTor does not claim authorship of that protocol implementation.

RusTor's original code is licensed under the [MIT License](LICENSE). Third-party dependencies retain their own licenses; release packaging must include their applicable notices.
