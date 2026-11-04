import assert from 'node:assert/strict';
import test from 'node:test';
import { normalizeReleases, preferredRelease, packageLinks } from '../releases.mjs';

const root = 'https://github.com/adityapandeydev/RusTor';
function release(overrides = {}) {
  return { id: 1, name: 'RusTor preview', tag_name: 'v0.1.0-preview.1', draft: false,
    prerelease: true, published_at: '2026-11-04T07:00:00Z', html_url: `${root}/releases/tag/v0.1.0-preview.1`,
    assets: [{ name: 'RusTor_0.1.0_x64-setup.exe', size: 5719626, state: 'uploaded',
      browser_download_url: `${root}/releases/download/v0.1.0-preview.1/RusTor_0.1.0_x64-setup.exe` }], ...overrides };
}

test('only published, valid releases reach the public archive; newer releases sort first', () => {
  const rows = normalizeReleases([release(), release({ id: 2, draft: true, name: 'PRIVATE DRAFT' }),
    release({ id: 3, published_at: null }), release({ id: 4, published_at: 'invalid' }),
    release({ id: 5, tag_name: 'v0.2.0', published_at: '2026-11-05T07:00:00Z' })]);
  assert.deepEqual(rows.map(row => row.id), [5, 1]);
  assert.ok(!JSON.stringify(rows).includes('PRIVATE DRAFT'));
});

test('release and asset links reject script URLs, another repository, credentials and lookalike origins', () => {
  for (const url of ['javascript:alert(1)', `${root}.evil/releases/tag/v1`, 'https://github.com/other/repo/releases/tag/v1',
    'https://user:secret@github.com/adityapandeydev/RusTor/releases/tag/v1']) {
    assert.equal(normalizeReleases([release({ html_url: url })]).length, 0);
  }
  const row = release();
  row.assets.push({ ...row.assets[0], browser_download_url: 'https://github.com.evil/file.exe' });
  row.assets.push({ ...row.assets[0], state: 'starter' });
  row.assets.push({ ...row.assets[0], size: -1 });
  assert.equal(normalizeReleases([row])[0].assets.length, 1);
});

test('platform matching keeps installers, standalone archives and Mac architectures distinct', () => {
  const names = ['RusTor_0.1.0_x64-setup.exe', 'RusTor_0.1.0_x64-standalone.zip',
    'RusTor_0.1.0_amd64.AppImage', 'RusTor_0.1.0_amd64.deb', 'RusTor_0.1.0_aarch64.dmg',
    'RusTor_0.1.0_x64.dmg', 'RusTor_0.1.0_release-info.zip', 'SHA256SUMS'];
  const row = normalizeReleases([release({ assets: names.map(name => ({ name, state: 'uploaded', size: 100,
    browser_download_url: `${root}/releases/download/v0.1.0-preview.1/${name}` })) })])[0];
  assert.equal(packageLinks(row, 'windows').length, 2);
  assert.equal(packageLinks(row, 'linux').length, 2);
  assert.match(packageLinks(row, 'mac-arm')[0].name, /aarch64/);
  assert.match(packageLinks(row, 'mac-intel')[0].name, /x64\.dmg/);
  assert.deepEqual(packageLinks(row, 'unsupported'), []);
});

test('the download selector prefers a stable release with packages, otherwise a preview', () => {
  const preview = normalizeReleases([release()])[0];
  const stable = normalizeReleases([release({ id: 7, prerelease: false })])[0];
  assert.equal(preferredRelease([preview, stable]), stable);
  assert.equal(preferredRelease([preview]), preview);
  assert.equal(preferredRelease([]), null);
  assert.equal(preferredRelease([{ ...stable, assets: [] }, preview]), preview);
});
