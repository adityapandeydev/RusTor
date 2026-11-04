# RusTor website

One page, plain HTML/CSS/JavaScript, with no browser runtime dependencies or TypeScript. Product screenshots use the current app components with disclosed example transfers. Font license and image origins are in `assets/`.

## Preview locally

Requires Node.js 24 and internet access to read public GitHub release metadata:

```sh
node web/build.mjs --serve
```

Open http://127.0.0.1:4175. Build output goes into a fresh system temporary directory, outside the repository. To choose another output directory:

```sh
node web/build.mjs --out /path/outside/the/repository
```

## Check

```sh
node --test web/tests/*.test.mjs
node --check web/site.mjs
```

## Publish

GitHub Pages must use **GitHub Actions** as its build source. Merging changes to `web/` or its workflow into main deploys the site. Publishing, editing, unpublishing or deleting a release also refreshes the site; a daily refresh covers asset changes. Manual refresh is available through Actions → Website → Run workflow on main.

The build reads every page of GitHub releases and exports only published releases and uploaded assets from this repository. It keeps preview/stable status and explicit Mac architecture choices. Drafts, API credentials, source code, tests, build scripts and internal documents are not copied into the published site. The build token is used only on the runner; the browser reads a static, public `releases.json` from the same site. Rendering uses DOM text nodes, not release-provided HTML.

Downloads stay on GitHub Releases. A visitor without JavaScript, or whose release data fails to load, can still reach the release page using the static fallback links. No analytics, forms, cookies or remote font service are included.
