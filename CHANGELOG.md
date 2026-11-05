# Changelog

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
