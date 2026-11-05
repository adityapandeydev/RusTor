import { preferredRelease, REPOSITORY } from './releases.mjs';

export const SITE_URL = 'https://adityapandeydev.github.io/RusTor/';
export const INDEXNOW_KEY = '1a644542c75e417aabb8157e94b32e40';

// JSON embedded in HTML must never let a release title close its script element.
export function htmlJson(value) {
  return JSON.stringify(value).replace(/</g, '\\u003c').replace(/>/g, '\\u003e').replace(/&/g, '\\u0026');
}

export function structuredData(releases) {
  const release = preferredRelease(releases);
  return {
    '@context': 'https://schema.org',
    '@graph': [
      { '@type': 'WebSite', '@id': `${SITE_URL}#website`, url: SITE_URL, name: 'RusTor', inLanguage: 'en' },
      { '@type': 'WebPage', '@id': `${SITE_URL}#webpage`, url: SITE_URL,
        name: 'RusTor — Free Torrent Client for Windows, Linux & macOS',
        isPartOf: { '@id': `${SITE_URL}#website` }, mainEntity: { '@id': `${SITE_URL}#software` }, inLanguage: 'en' },
      { '@type': 'SoftwareApplication', '@id': `${SITE_URL}#software`, name: 'RusTor', url: SITE_URL,
        description: 'A free, open source torrent client built with Rust. Search, select files and manage downloads with speed limits and saved sessions.',
        applicationCategory: 'UtilitiesApplication', operatingSystem: 'Windows, Linux, macOS',
        license: `${REPOSITORY}/blob/main/LICENSE`, isAccessibleForFree: true,
        offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD', url: `${SITE_URL}#downloads` },
        author: { '@type': 'Person', name: 'Aditya Pandey', url: 'https://github.com/adityapandeydev' },
        screenshot: `${SITE_URL}assets/downloads.jpg`, sameAs: REPOSITORY,
        ...(release ? { softwareVersion: release.tag.replace(/^v/, ''), downloadUrl: release.url, releaseNotes: release.url } : {}),
      },
    ],
  };
}

// This is one page: section fragments and binary downloads are not separate pages.
export const sitemap = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>${SITE_URL}</loc></url>
</urlset>
`;
