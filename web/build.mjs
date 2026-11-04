import { cp, mkdir, readFile, writeFile, mkdtemp, realpath } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, extname, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { normalizeReleases } from './releases.mjs';

const source = dirname(fileURLToPath(import.meta.url));

export async function fetchReleases(request = fetch, token) {
  const rows = [];
  for (let page = 1; ; page++) {
    const response = await request(`https://api.github.com/repos/adityapandeydev/RusTor/releases?per_page=100&page=${page}`, {
      headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'RusTor-website',
        ...(token ? { Authorization: `Bearer ${token}` } : {}) }, signal: AbortSignal.timeout(30000),
    });
    if (!response.ok) throw new Error(`Could not load GitHub releases (HTTP ${response.status}).`);
    const batch = await response.json();
    if (!Array.isArray(batch)) throw new Error('Invalid GitHub release response.');
    if (batch.length && page > 1 && rows.some(row => row.id === batch[0].id))
      throw new Error('GitHub release pagination repeated a page.');
    rows.push(...batch);
    if (batch.length < 100) return normalizeReleases(rows);
  }
}

export async function build(output, releases) {
  const fromSource = relative(source, output);
  if (!fromSource || (!isAbsolute(fromSource) && !fromSource.startsWith('..')))
    throw new Error('Choose an output directory outside web/ to keep build files separate.');
  await mkdir(output, { recursive: true });
  for (const name of ['index.html', 'styles.css', 'site.mjs', 'releases.mjs', 'assets', '.nojekyll'])
    await cp(join(source, name), join(output, name), { recursive: true });
  await writeFile(join(output, 'releases.json'), JSON.stringify({ releases }, null, 2) + '\n');
  return output;
}

async function main() {
  const args = process.argv.slice(2);
  if (args.some(arg => !['--out', '--serve'].includes(arg) && args[args.indexOf(arg) - 1] !== '--out'))
    throw new Error('Usage: node web/build.mjs [--out DIRECTORY] [--serve]');
  const outIndex = args.indexOf('--out');
  if (outIndex !== -1 && (!args[outIndex + 1] || args[outIndex + 1].startsWith('--')))
    throw new Error('--out needs a directory.');
  const output = outIndex === -1 ? await mkdtemp(join(tmpdir(), 'rustor-web-')) : resolve(args[outIndex + 1]);
  const releases = await fetchReleases(fetch, process.env.GITHUB_TOKEN);
  await build(output, releases);
  console.log(`Built ${releases.length} public releases into ${output}`);
  if (!args.includes('--serve')) return;
  const root = await realpath(output);
  const types = { '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.mjs': 'text/javascript',
    '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.png': 'image/png', '.webp': 'image/webp' };
  createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
      const file = resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname));
      const inside = relative(root, file);
      if (inside.startsWith('..') || isAbsolute(inside)) throw new Error('Invalid path');
      const data = await readFile(file);
      response.writeHead(200, { 'Content-Type': types[extname(file)] || 'application/octet-stream' });
      response.end(data);
    } catch { response.writeHead(404); response.end('Not found'); }
  }).listen(4175, '127.0.0.1', () => console.log('Site preview: http://127.0.0.1:4175'));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
