export const MEASUREMENT_ID = 'G-TVDKMQ11N1';
const CHOICE_KEY = 'rustor.analytics.v1';
const HOST = 'adityapandeydev.github.io';
const SITE_PATH = '/RusTor/';

export function downloadDetails(href, location) {
  try {
    const url = new URL(href);
    if (url.origin !== 'https://github.com' || url.search || url.hash || url.username || url.password) return null;
    const match = url.pathname.match(/^\/adityapandeydev\/RusTor\/releases\/download\/([^/]+)\/([^/]+)$/);
    if (!match) return null;
    const file = decodeURIComponent(match[2]);
    let platform;
    if (/_x64-(setup\.exe|standalone\.zip)$/i.test(file)) platform = 'windows';
    else if (/_amd64\.(AppImage|deb)$/i.test(file)) platform = 'linux';
    else if (/_aarch64\.dmg$/i.test(file)) platform = 'mac-arm';
    else if (/_x64\.dmg$/i.test(file)) platform = 'mac-intel';
    else return null;
    return { platform, version: decodeURIComponent(match[1]), file_name: file,
      button_location: location, link_url: url.href };
  } catch { return null; }
}

export function createAnalytics(browser) {
  let started = false;
  let currentChoice;
  function choice() {
    if (currentChoice) return currentChoice;
    try {
      const saved = browser.localStorage.getItem(CHOICE_KEY);
      return saved === 'accepted' || saved === 'declined' ? saved : null;
    } catch { return null; }
  }
  function start() {
    if (started || choice() !== 'accepted' || browser.location.hostname !== HOST ||
      !browser.location.pathname.startsWith(SITE_PATH)) return;
    started = true;
    browser[`ga-disable-${MEASUREMENT_ID}`] = false;
    browser.dataLayer = browser.dataLayer || [];
    browser.gtag = function () { browser.dataLayer.push(arguments); };
    browser.gtag('consent', 'default', { analytics_storage: 'granted', ad_storage: 'denied',
      ad_user_data: 'denied', ad_personalization: 'denied' });
    browser.gtag('js', new Date());
    browser.gtag('config', MEASUREMENT_ID, { cookie_path: SITE_PATH, cookie_domain: 'none',
      allow_google_signals: false, allow_ad_personalization_signals: false,
      page_location: `${browser.location.origin}${SITE_PATH}` });
    const script = browser.document.createElement('script');
    script.async = true;
    script.src = `https://www.googletagmanager.com/gtag/js?id=${MEASUREMENT_ID}`;
    browser.document.head.append(script);
  }
  function choose(value) {
    if (value !== 'accepted' && value !== 'declined') return;
    currentChoice = value;
    try { browser.localStorage.setItem(CHOICE_KEY, value); } catch { /* Choice still applies for this visit. */ }
    if (value === 'accepted') start();
    else if (started) {
      browser[`ga-disable-${MEASUREMENT_ID}`] = true;
      for (const cookie of browser.document.cookie.split(';')) {
        const name = cookie.trim().split('=')[0];
        if (name === '_ga' || name === `_ga_${MEASUREMENT_ID.slice(2)}`)
          browser.document.cookie = `${name}=; Max-Age=0; Path=${SITE_PATH}; SameSite=Lax`;
      }
      // Unload Google's listeners too; the saved choice prevents loading on return.
      browser.location.reload();
    }
  }
  function download(href, location) {
    const details = downloadDetails(href, location);
    if (details && started && choice() === 'accepted')
      browser.gtag('event', 'download_click', { ...details, send_to: MEASUREMENT_ID });
  }
  return { choice, start, choose, download };
}

let client;
export function analytics() {
  return client ??= createAnalytics(window);
}
