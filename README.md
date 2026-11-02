# RusTor

A planned Rust-powered desktop BitTorrent application, with a React and TypeScript interface delivered through Tauri 2.

Status: upstream research in progress. No application or release is implemented yet.

Frontend: React, TypeScript, Bun, Vite, and Tailwind CSS v4. The backend will port Torlink's useful non-TUI behavior into Rust, using a proven torrent protocol library where suitable.

## Acknowledgements

RusTor is an independent project inspired by [Torlink](https://github.com/baairon/torlink), developed by bairon.dev and its contributors. Its search, torrent-management, and persistence behavior provides the foundation for our planned Rust port. RusTor is not an official Torlink release and does not develop or ship its TUI.

Torlink is licensed under the MIT License, copyright 2026 bairon.dev. No upstream application code has been copied into this repository at this stage. As code is adapted, this section will identify the reused portions and applicable notices will be preserved. RusTor's project license will be selected after the dependency and reuse assessment; no release license is claimed yet.
