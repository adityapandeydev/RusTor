import { createElement } from 'react';
import { renderToString } from 'react-dom/server';
import { build as viteBuild } from 'vite';
import { readFile, writeFile } from 'node:fs/promises';
import { join, resolve, relative, isAbsolute } from 'node:path';
import App from './src/App.jsx';
import { fetchReleases } from './release-feed.mjs';
import { htmlJson, structuredData, sitemap, INDEXNOW_KEY } from './seo.mjs';

const args = process.argv.slice(2);
if (args.length && (args.length !== 2 || args[0] !== '--out' || !args[1] || args[1].startsWith('--')))
  throw new Error('Usage: bun build.mjs [--out DIRECTORY]');
const outDir = args.length ? resolve(args[1]) : join(import.meta.dirname, 'dist');
const fromSource = relative(import.meta.dirname, outDir);
if (!fromSource || (!isAbsolute(fromSource) && !fromSource.startsWith('..') && fromSource !== 'dist'))
  throw new Error('Choose web/dist or an output directory outside web/ to keep source files separate.');
const releases = await fetchReleases(fetch, process.env.GITHUB_TOKEN);
await viteBuild({ configFile: join(import.meta.dirname, 'vite.config.js'), build: { outDir, emptyOutDir: false } });
const entry = join(outDir, 'index.html');
const template = await readFile(entry, 'utf8');
await writeFile(entry, template
  .replace('<!--site-data-->', () => `<script type="application/ld+json">${htmlJson(structuredData(releases))}</script><script id="release-data" type="application/json">${htmlJson(releases)}</script>`)
  .replace('<!--app-html-->', () => renderToString(createElement(App, { initialReleases: releases }))));
await writeFile(join(outDir, 'releases.json'), JSON.stringify({ releases }, null, 2) + '\n');
await writeFile(join(outDir, 'sitemap.xml'), sitemap);
await writeFile(join(outDir, `${INDEXNOW_KEY}.txt`), INDEXNOW_KEY);
console.log(`Built ${releases.length} public releases into ${outDir}`);
