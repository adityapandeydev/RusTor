import assert from 'node:assert/strict';
import test from 'node:test';
import { fetchReleases } from '../release-feed.mjs';

const publicRelease = id => ({ id, name: `Release ${id}`, tag_name: `v${id}`, draft: false, prerelease: false,
  html_url: `https://github.com/adityapandeydev/RusTor/releases/tag/v${id}`, published_at: '2026-11-04T07:00:00Z', assets: [] });

test('release generation follows pagination and never exports drafts or API-only fields', async () => {
  const calls = [];
  const releases = await fetchReleases(async (url, options) => {
    calls.push(url);
    assert.equal(options.headers.Authorization, 'Bearer build-only-secret');
    const batch = calls.length === 1 ? Array.from({ length: 100 }, (_, i) => publicRelease(i + 1)) :
      [{ ...publicRelease(101), draft: true, name: 'PRIVATE DRAFT' }, { ...publicRelease(102), author: 'private-field' }];
    return { ok: true, json: async () => batch };
  }, 'build-only-secret');
  assert.equal(calls.length, 2);
  assert.match(calls[1], /page=2$/);
  assert.equal(releases.length, 101);
  assert.ok(!JSON.stringify(releases).match(/PRIVATE DRAFT|build-only-secret|private-field/));
});

test('API failures or broken pagination fail the build rather than publish a partial archive', async () => {
  await assert.rejects(fetchReleases(async () => ({ ok: false, status: 403 })), /HTTP 403/);
  await assert.rejects(fetchReleases(async () => ({ ok: true, json: async () => ({ message: 'error' }) })), /Invalid/);
  const repeated = Array.from({ length: 100 }, (_, i) => publicRelease(i + 1));
  await assert.rejects(fetchReleases(async () => ({ ok: true, json: async () => repeated })), /repeated a page/);
});
