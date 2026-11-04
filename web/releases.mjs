const REPOSITORY = 'https://github.com/adityapandeydev/RusTor';

function trustedLink(value, path) {
  try {
    const url = new URL(value);
    return url.origin === 'https://github.com' && !url.username && !url.password &&
      !url.search && !url.hash && url.pathname.startsWith(`/adityapandeydev/RusTor/releases/${path}/`)
      ? url.href : null;
  } catch { return null; }
}

export function normalizeReleases(input) {
  if (!Array.isArray(input)) throw new Error('GitHub returned an invalid release list.');
  const releases = new Map();
  for (const row of input) {
    const url = trustedLink(row.html_url, 'tag');
    const date = Date.parse(row.published_at);
    if (row.draft !== false || !row.published_at || !Number.isFinite(date) || !url ||
      !Number.isSafeInteger(row.id) || typeof row.tag_name !== 'string') continue;
    const assets = (Array.isArray(row.assets) ? row.assets : []).flatMap(asset => {
      const download = trustedLink(asset.browser_download_url, 'download');
      if (!download || asset.state !== 'uploaded' || !Number.isSafeInteger(asset.size) ||
        asset.size <= 0 || typeof asset.name !== 'string' || asset.name.length > 255) return [];
      return [{ name: asset.name, size: asset.size, url: download }];
    });
    releases.set(row.id, { id: row.id, name: String(row.name || row.tag_name).slice(0, 160),
      tag: row.tag_name.slice(0, 120), preview: row.prerelease === true,
      published: new Date(date).toISOString(), url, assets });
  }
  return [...releases.values()].sort((a, b) => Date.parse(b.published) - Date.parse(a.published));
}

const PACKAGES = {
  windows: [[/_x64-setup\.exe$/i, 'Windows installer'], [/_x64-standalone\.zip$/i, 'Standalone ZIP']],
  linux: [[/_amd64\.AppImage$/i, 'AppImage'], [/_amd64\.deb$/i, 'Debian package']],
  'mac-arm': [[/_aarch64\.dmg$/i, 'Apple Silicon DMG']],
  'mac-intel': [[/_x64\.dmg$/i, 'Intel DMG']],
};

export function packageLinks(release, platform) {
  if (!release) return [];
  return (PACKAGES[platform] || []).flatMap(([pattern, label]) => {
    const asset = release.assets.find(item => pattern.test(item.name));
    return asset ? [{ ...asset, label }] : [];
  });
}

export function preferredRelease(releases) {
  const hasPackages = release => Object.keys(PACKAGES).some(key => packageLinks(release, key).length);
  return releases.find(release => !release.preview && hasPackages(release)) || releases.find(hasPackages) || null;
}

export function fileSize(bytes) {
  return `${new Intl.NumberFormat('en', { maximumFractionDigits: 1 }).format(bytes / 1048576)} MB`;
}

export { REPOSITORY };
