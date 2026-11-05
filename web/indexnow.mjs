import { INDEXNOW_KEY, SITE_URL } from './seo.mjs';

// Run only after deployment: engines must be able to fetch the public key first.
const keyLocation = `${SITE_URL}${INDEXNOW_KEY}.txt`;
const key = await fetch(keyLocation, { signal: AbortSignal.timeout(30000) });
if (!key.ok || (await key.text()).trim() !== INDEXNOW_KEY)
  throw new Error('The deployed IndexNow key is not available yet. Retry after Pages has refreshed.');
const response = await fetch('https://api.indexnow.org/indexnow', {
  method: 'POST', headers: { 'Content-Type': 'application/json; charset=utf-8' },
  body: JSON.stringify({ host: new URL(SITE_URL).host, key: INDEXNOW_KEY, keyLocation, urlList: [SITE_URL] }),
  signal: AbortSignal.timeout(30000),
});
if (response.status !== 200 && response.status !== 202)
  throw new Error(`IndexNow submission failed (HTTP ${response.status}).`);
console.log(response.status === 202 ? 'IndexNow received the URL; key validation is pending (HTTP 202).' :
  'IndexNow received the URL (HTTP 200). Indexing is decided by the search engine.');
