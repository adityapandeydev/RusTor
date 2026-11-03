# RusTor

A Rust-powered desktop BitTorrent application in development, with a React and TypeScript interface delivered through Tauri 2.

Status: the native engine and first desktop Downloads screen are implemented. Local transfer and service tests run on Windows. Discover registers all ten Torlink source entries through the Rust backend. Some upstream endpoints are currently unavailable or require sign-in, so implemented coverage does not guarantee every source will return results. Advanced queue management and release installers remain pending.

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

## Search foundation

The `rustor-search` crate provides validated result contracts and an asynchronous provider interface independent of the torrent engine. It normalizes hex/base32 v1 identities, deduplicates torrents while preserving the winning magnet's existing parameters and merging trackers, retains source attribution and missing metadata, distinguishes unknown swarm health from known zero seeders, and supports category selection, text matching, and explicit ordering. Equal-health merges use deterministic tie breakers instead of provider completion order.

The service publishes the latest progressive snapshot, isolates provider failures/panics/timeouts, and applies per-provider and overall deadlines. Starting a new search cancels its predecessor; dropping its handle aborts outstanding work. Concurrency is shared across superseded searches. Crate defaults are four active provider tasks, 15-second provider and 30-second overall deadlines, 100 candidates per provider, and 500 displayed results. The desktop uses 45-second provider and 90-second overall deadlines to accommodate ten providers and bounded multi-page/detail requests; concurrency and result caps remain unchanged. Registration is capped at 32 providers, accepted candidates at 3,200, magnets at 16 KiB, and merged trackers at 64; truncation/rejection is reported. Provider implementations must also bound their own HTTP response decoding and use cooperative, nonblocking futures.

### Nyaa RSS provider

`providers::NyaaProvider` supplies live metadata through Nyaa's public HTTPS RSS feed, using the same all-category feed query as Torlink and registering under Anime. It maps namespaced infohashes, sizes, dates, and available swarm counts into the validated search pipeline. Magnets include encoded titles and the public tracker defaults ported from Torlink; search does not contact those trackers or fetch torrent payloads. Coverage is the first RSS page, not Nyaa's entire archive.

Requests use a fixed origin, verified TLS, no redirects or automatic retries, a 12-second HTTP deadline, and two seconds between request starts per provider instance. HTTP 429 applies a cooldown (60 seconds if no valid `Retry-After`); HTTP 503 also honors `Retry-After`. XML decoding is limited to 2 MiB, 20,000 nodes, and 101 candidates (one overflow candidate lets the service report its 100-row cap). Compressed bodies and non-XML responses are rejected; XML entity declarations are disabled. Unknown counts stay unknown and malformed candidates are counted by search validation.

The Discover screen connects to this adapter through Tauri commands and bounded progressive snapshots. It supports query submission, cancellation, category filters, relevance/seeders/size/date ordering, hiding known zero-seeder results while retaining unknown health, and paginated results. Choosing a result opens the existing add-torrent file-selection flow; it does not start a transfer automatically. New searches replace old ones, stale responses are ignored, and shutdown aborts outstanding search work. Queries and provider URLs are handled in Rust; the frontend cannot choose arbitrary provider endpoints.

All, Movies, TV, Games, and Anime filters are present. Category availability now follows the backend's selected provider entries rather than an Anime-only UI assumption. Categories with no connected adapter explain that their sources are pending; failures from selected providers remain visible without hiding successful results.

### JSON providers and current coverage

`providers::JsonProvider` ports three additional Torlink adapters:

| Provider | Categories | Behavior and limits |
| --- | --- | --- |
| YTS | Movies | Requests 50 movies and maps quality/type variants, sizes and swarm counts. Tries Torlink's configured `yts.mx`, `yts.am`, and `yts.rs` hosts in order, with four-second attempts inside the service deadline. A rate-limit response stops fallback. |
| BitTorrented | Movies, TV | Searches video metadata only, with at least three query characters and a 50-result API request. The endpoint returned HTTP 401 with an invite-only/sign-in requirement during validation; this client does not implement authentication, so this source currently reports unavailable. |
| SubsPlease | Anime | Uses latest/search metadata, selects a valid 1080/720/480 release in that preference order, and keeps health unknown because the API has no swarm counts. It preserves supported magnet parameters and removes WS/WSS tracker parameters unsupported by the native engine. |

All adapters share verified-TLS transport with no redirects or automatic retries, two-second request spacing per provider instance, Retry-After cooldowns, and a 2 MiB body cap. JSON decoding retains the parser's recursion limit, limits mapping to 101 candidates, and bounds copied fields. Each provider owns its own rate gate. SubsPlease currently labels valid JSON as `text/html`; that fixed endpoint alone accepts this content type, and the body must still decode as the expected JSON envelope. HTML challenge pages are rejected. Invalid candidates and truncated results are reported by the existing search pipeline.

### Remaining provider families

| Provider entry | Categories | Behavior and bounds |
| --- | --- | --- |
| FitGirl | Games | WordPress RSS with relevance ordering for nonempty queries; reads up to three pages when page one has at least ten items. Extracts a magnet from the feed's own HTML content without visiting post links. Missing health and size remain unknown; deeper-page failures retain earlier results. |
| TPB Movies / TPB TV | Movies / TV | Separate apibay category sets and top-100 browse endpoints. A search's no-result sentinel triggers one request with explicit `cat=0`, matching Torlink's alternate cache-key behavior. Metadata strings are checked before numeric conversion. |
| EZTV | TV | Empty browse fetches one 100-row page. Text search scans up to ten recent pages, caches that index for five minutes, matches every query token against titles/filenames, and looks up at most two matched IMDb show IDs. The index has a 30-second budget; deeper/catalogue failures can narrow coverage while retaining usable results. Hash duplicates are removed and the highest-seeded results are capped. |
| 1337x Movies / 1337x TV | Movies / TV | Uses Torlink's four configured hosts, category routes and stop-word matching. Fetches at most four matching detail pages in seeder order, extracts magnets and upload dates, and remembers the working host per adapter. Detail URLs must stay on the chosen origin and a numeric `/torrent/` route; queries, fragments and external links are rejected. |

**All ten entries across eight sites in the pinned Torlink registry are now implemented and registered.** None remain unported from that registry. During metadata validation, FitGirl and both TPB entries returned validated results. EZTV and 1337x were unavailable from this environment, while BitTorrented's endpoint remains gated by its sign-in requirement. Implemented adapters therefore do not guarantee live access, identical result counts, or access to each site's full archive. Query caching beyond EZTV's recent-show index remains follow-up work.

HTML is parsed as data with a pinned browser-grade parser, never rendered or executed. Response decoding remains capped at 2 MiB; XML/HTML structure, mapped fields, requests and result counts are bounded. Optional page/detail failures can narrow a provider's coverage; first-page failures remain visible as source failures. No CAPTCHA solving, browser-cookie reuse, or authentication bypass is implemented.

Automated tests use synthetic metadata and loopback HTTP, including cancellation, query encoding, bounds, mirror fallback, rate limits, unknown health and category availability. The ignored `nyaa_live`, `json_live`, and `remaining_live` integration tests provide opt-in public metadata checks and availability diagnostics. Live checks for YTS and SubsPlease returned validated results; endpoint availability can vary. Search does not fetch torrent payloads.

## Desktop development

The Downloads screen connects to the real Rust session through Tauri commands and live snapshots. It includes magnet/infohash entry, native `.torrent` and destination pickers, transfer filters, progress and speed reporting, details, and pause/resume, stop/remove, and failed-transfer retry controls. Failed transfers show recovery guidance and expandable engine error details without opening the row. Retry rechecks existing data in place and retains the destination and file selection. The × action stops downloading or seeding and removes the torrent from the saved session while keeping complete and partial files on disk. Dark mode uses navy surfaces with peach accents; a matching warm light mode is available from the header toggle, and the appearance preference is saved locally. The shared interface uses clearer type hierarchy and roomier rows, with animated button presses, page/panel entrances, and real transfer progress. Reduced-motion preferences disable those animations. The add flow loads metadata first, then lets you choose files and review their total size before starting. Large lists support search and pagination. Selections survive session restoration; skipped files can still receive data from shared torrent pieces. New previews currently support up to 10,000 files. File reads stay in Rust and use handles returned by the native picker; the frontend cannot submit an arbitrary file path. There is no end-user CLI or TUI.

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

Searchable-content coverage expands through provider integrations; the torrent library does not supply a catalog. Drag-and-drop, notifications, payload deletion, changing file selection after addition, and download queue policies are follow-up work.

## Acknowledgements

RusTor is an independent project inspired by [Torlink](https://github.com/baairon/torlink), developed by bairon.dev and its contributors. Its search, torrent-management, and persistence behavior provides the foundation for our planned Rust port. RusTor is not an official Torlink release and does not develop or ship its TUI.

Torlink is licensed under the MIT License, copyright 2026 bairon.dev. RusTor's input normalization and lifecycle behavior are informed by its magnet, queue, and persistence implementations. The Rust search contracts, tracker-preserving duplicate merge, ordering, and token matching in `crates/rustor-search` port behavior from Torlink's [source types](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/types.ts), [deduplication](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/ui/dedupe.ts), [sorting](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/ui/sort.ts), [filtering](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/ui/filter.ts), [magnet helpers](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/magnet.ts), [Nyaa adapter](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/nyaa.ts), and [format helpers](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/util/format.ts) at revision `df059311d0898b7245dd842153c1ff7c892275ee`. The JSON adapters also port the [YTS](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/yts.ts), [BitTorrented](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/bittorrented.ts), and [SubsPlease](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/subsplease.ts) mappings from that same revision. The remaining families port [FitGirl](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/fitgirl.ts) and [RSS](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/rss.ts), [TPB](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/piratebay.ts), [EZTV](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/eztv.ts), and [1337x](https://github.com/baairon/torlink/blob/df059311d0898b7245dd842153c1ff7c892275ee/src/sources/x1337.ts) behavior from the same revision. Its TypeScript source and terminal UI are not distributed. The applicable upstream notice is retained here:

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
