import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { JSDOM } from 'jsdom';
import { SITE_URL, INDEXNOW_KEY, htmlJson, sitemap, structuredData } from '../seo.mjs';

test('canonical, social previews and sitemap describe the single public page', async () => {
  const { document } = new JSDOM(await readFile(new URL('../index.html', import.meta.url), 'utf8')).window;
  assert.equal(document.querySelector('link[rel="canonical"]').href, SITE_URL);
  assert.match(document.title, /RusTor.*Free Torrent Client.*Windows.*Linux.*macOS/);
  assert.equal(document.querySelector('meta[property="og:url"]').content, SITE_URL);
  assert.equal(document.querySelector('meta[name="robots"]').content, 'index, follow, max-image-preview:large');
  for (const selector of ['meta[property="og:image"]', 'meta[name="twitter:image"]'])
    assert.equal(document.querySelector(selector).content, `${SITE_URL}assets/downloads.jpg`);
  assert.ok(document.querySelector('meta[property="og:image:alt"]').content);
  const xml = new JSDOM(sitemap, { contentType: 'application/xml' }).window.document;
  assert.equal(xml.querySelector('parsererror'), null);
  assert.equal(xml.querySelectorAll('loc').length, 1);
  assert.equal(xml.querySelector('loc').textContent, SITE_URL);
  assert.equal(xml.querySelector('lastmod'), null); // Scheduled builds must not pretend content changed.
  assert.match(INDEXNOW_KEY, /^[a-zA-Z0-9-]{8,128}$/);
});

test('software data is factual, version-aware and safely embedded in HTML', () => {
  const release = { tag: 'v0.1.2', preview: false, url: 'https://github.com/adityapandeydev/RusTor/releases/tag/v0.1.2',
    assets: [{ name: 'RusTor_0.1.2_x64-setup.exe', url: 'https://github.com/adityapandeydev/RusTor/releases/download/v0.1.2/RusTor_0.1.2_x64-setup.exe' }] };
  const app = structuredData([release])['@graph'].find(node => node['@type'] === 'SoftwareApplication');
  assert.equal(app.softwareVersion, '0.1.2');
  assert.equal(app.offers.price, '0');
  assert.equal(app.aggregateRating, undefined);
  assert.equal(structuredData([])['@graph'][2].softwareVersion, undefined);
  const unsafe = [{ name: '</script><script>alert("x")</script> & $&' }];
  const json = htmlJson(unsafe);
  assert.ok(!json.includes('<') && !json.includes('&'));
  const doc = new JSDOM(`<script type="application/json">${json}</script>`).window.document;
  assert.equal(doc.querySelectorAll('script').length, 1);
  assert.deepEqual(JSON.parse(doc.querySelector('script').textContent), unsafe);
});
