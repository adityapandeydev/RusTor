import { normalizeReleases } from './releases.mjs';

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

