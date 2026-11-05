# RusTor website

React JSX, Bun, Vite and Tailwind CSS v4. This workspace builds the GitHub Pages website; it does not change or build the desktop app.

From the repository root:

```sh
bun install --frozen-lockfile
bun run web:dev
bun run web:test
bun run web:build
bun run --cwd web preview
```

The dev server serves the JSX directly. The release build fetches all public GitHub releases, compiles through Vite, and pre-renders the same React components into `web/dist/`. React hydrates that markup in the browser. The page and GitHub download fallbacks remain available without JavaScript. To build into a separate directory, use `bun run web:build --out /absolute/output/path`.

`src/App.jsx` contains the page and selection state; `src/Downloads.jsx` contains download links and the release archive; `src/motion.js` preserves the existing animations and reduced-motion handling. Tailwind v4 is configured in `src/styles.css` and `vite.config.js`. Preflight is intentionally omitted to preserve the established browser defaults and exact visual design. The remaining custom CSS holds the original responsive geometry and authored motion.

`public/assets/` contains the unchanged product captures, font and logo. `release-feed.mjs` is build-only GitHub access; `releases.mjs` validates public metadata and selects platform packages. `GITHUB_TOKEN`, when provided in CI, stays on the build side and is never bundled.

The Pages workflow deploys on website changes, public release events, manual dispatch and its existing schedule. Relative Vite asset paths support the `/RusTor/` project URL. Do not commit generated `dist/` output.

Product captures use disclosed example transfers; their origins and the font license are preserved in `public/assets/`. The feed retains explicit preview/stable and Mac architecture choices. Release titles render as React text, never as release-provided HTML. Drafts, credentials, source, tests, build scripts and internal documents are not published. No analytics, forms, cookies or remote font service are added.
