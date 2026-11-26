# Changelog

## 0.2.2

### Fixed

- Look up search and artwork hosts with encrypted DNS-over-HTTPS (Cloudflare, with Google as a fallback, and system DNS if both are unreachable), which can improve source availability on networks with DNS-level filtering. Only fixed host names are sent.
- Update YTS to its current API hosts (`movies-api.accel.li`, `yts.pw`, `yts.rs`) and cover image hosts; retire `yts.mx` and `yts.am`.
- Explain source failures in Discover: could not connect (a VPN or another network may help), blocked by the site's bot check, moved, or unavailable. Sources with several mirrors report the most specific reason.
- Correct the README's description of tray mode, in-app updates and where to download releases.

### Removed

- BitTorrented search, whose API now requires an invitation-only account.

## 0.2.1

- Provider artwork in Discover and Downloads, with lazy loading and a local thumbnail cache.
- Keep the current list layout, theme, sorting and transfer controls. Missing artwork uses a file placeholder.
- Fetch only approved provider image origins through Rust; reject redirects, oversized images and unsupported formats. No additional API account is needed.
- Fill missing movie, TV and anime covers using exact metadata matches; cache lookup results, coalesce episodes/qualities and throttle requests. Fix support for current YTS image URLs.

## 0.2.0

### Added

- Optional tray mode with Open, Pause all, Resume all and Quit controls.
- Optional start at login and minimized login launch, with single-instance protection.
- Manual in-app update checks, download progress, signature verification and explicit install/restart confirmation.
- Checked update shutdown that verifies and flushes saved recovery data before stopping transfers; failed saves cancel installation.
- Dedicated updater signatures and a complete Windows/macOS/Linux update manifest. Windows publisher signing and macOS notarization remain unconfigured.

### Upgrade

- Install 0.2.0 over the existing installation without uninstalling it. Versions 0.1.0–0.1.2 have no in-app updater and require this manual upgrade.
- Subsequent in-app updates support installed Windows NSIS builds, macOS apps and Linux AppImages. Standalone Windows ZIP and Debian packages use manual upgrades.
- Native macOS/Linux desktop behavior and true old-to-new installer upgrades still need validation. Sleep, logout and shutdown interrupt transfers.

## 0.1.2

### Changed

- Refresh both app themes with deep teal, pearl and lime, preserving the existing layout and download behavior.
- Rebuild the website with an animated landing graphic, interactive download buttons, complete app previews and a public release archive.


## 0.1.1

### Fixed

- Make the search-cancellation UI test wait for an explicit cancellation instead of stopping itself during polling.
- Reduce test-worker CPU contention in native packaging checks while preserving test deadlines and assertions.
- Wait for the loopback seeder to observe a paused connection closing before testing resume, instead of racing socket teardown.
- Align website feature text with complete product screenshots on desktop and mobile.

### Changed

- Report the frontend package version in the app footer, so the installed app and its packages stay in sync.
