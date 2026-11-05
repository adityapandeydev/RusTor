import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { JSDOM } from 'jsdom';
import AnalyticsConsent from '../src/AnalyticsConsent.jsx';
import { createAnalytics, downloadDetails, MEASUREMENT_ID } from '../src/analytics.js';
import { analytics } from '../src/analytics.js';
import { DownloadLink, ReleaseArchive } from '../src/Downloads.jsx';

afterEach(() => { cleanup(); vi.restoreAllMocks(); localStorage.clear(); });
const file = 'https://github.com/adityapandeydev/RusTor/releases/download/v0.1.2/RusTor_0.1.2_x64-setup.exe';
const production = () => new JSDOM('', { url: 'https://adityapandeydev.github.io/RusTor/' }).window;
const commands = browser => (browser.dataLayer || []).map(args => Array.from(args));

it('makes no Google requests or queued events until explicit consent and remembers declines', () => {
  const browser = production();
  const client = createAnalytics(browser);
  client.start();
  client.download(file, 'hero');
  expect(browser.document.scripts.length).toBe(0);
  expect(browser.dataLayer).toBeUndefined();
  client.choose('declined');
  expect(createAnalytics(browser).choice()).toBe('declined');
  expect(browser.document.scripts.length).toBe(0);
  browser.close();
});

it('loads the direct GA4 tag once, denies advertising and sends one scoped click per activation', () => {
  const browser = production();
  const client = createAnalytics(browser);
  client.choose('accepted');
  client.start();
  client.choose('accepted');
  expect(browser.document.scripts.length).toBe(1);
  expect(browser.document.scripts[0].src).toBe(`https://www.googletagmanager.com/gtag/js?id=${MEASUREMENT_ID}`);
  expect(commands(browser)[0]).toEqual(['consent', 'default', { analytics_storage: 'granted',
    ad_storage: 'denied', ad_user_data: 'denied', ad_personalization: 'denied' }]);
  const config = commands(browser).find(row => row[0] === 'config');
  expect(config[2]).toMatchObject({ cookie_path: '/RusTor/', cookie_domain: 'none', allow_google_signals: false });
  client.download(file, 'hero');
  client.download('https://github.com/adityapandeydev/RusTor/releases', 'hero');
  const events = commands(browser).filter(row => row[0] === 'event');
  expect(events).toHaveLength(1);
  expect(events[0]).toEqual(['event', 'download_click', { platform: 'windows', version: 'v0.1.2',
    file_name: 'RusTor_0.1.2_x64-setup.exe', button_location: 'hero', link_url: file, send_to: MEASUREMENT_ID }]);
  browser.close();
});

it('resumes accepted choices but suppresses analytics on local previews and other projects', () => {
  for (const url of ['http://localhost:4175/RusTor/', 'https://adityapandeydev.github.io/Other/']) {
    const browser = new JSDOM('', { url }).window;
    createAnalytics(browser).choose('accepted');
    expect(browser.document.scripts.length).toBe(0);
    expect(browser.dataLayer).toBeUndefined();
    browser.close();
  }
  const browser = production();
  createAnalytics(browser).choose('accepted');
  browser.document.querySelector('script').remove();
  const nextPage = createAnalytics(browser);
  expect(nextPage.choice()).toBe('accepted');
  nextPage.start();
  expect(browser.document.scripts.length).toBe(1);
  browser.close();
});

it('withdraws consent, deletes only project GA cookies and reloads to unload the tag', () => {
  const browser = production();
  const reload = vi.fn();
  const client = createAnalytics({ location: { hostname: browser.location.hostname, pathname: '/RusTor/',
    origin: browser.location.origin, reload }, document: browser.document, localStorage: browser.localStorage });
  client.choose('accepted');
  browser.document.cookie = '_ga=test; Path=/RusTor/';
  browser.document.cookie = '_ga_TVDKMQ11N1=test; Path=/RusTor/';
  browser.document.cookie = 'unrelated=keep; Path=/RusTor/';
  client.choose('declined');
  expect(client.choice()).toBe('declined');
  expect(browser.document.cookie).toBe('unrelated=keep');
  expect(reload).toHaveBeenCalledOnce();
  browser.close();
});

it('records all installer formats and excludes navigation, checksums, source bundles and untrusted URLs', () => {
  const base = 'https://github.com/adityapandeydev/RusTor/releases/download/v0.1.2/';
  for (const [suffix, platform] of [['x64-setup.exe', 'windows'], ['x64-standalone.zip', 'windows'],
    ['amd64.AppImage', 'linux'], ['amd64.deb', 'linux'], ['aarch64.dmg', 'mac-arm'], ['x64.dmg', 'mac-intel']])
    expect(downloadDetails(`${base}RusTor_0.1.2_${suffix}`, 'release-archive').platform).toBe(platform);
  for (const url of ['#downloads', `${base}SHA256SUMS`, `${base}release-info.zip`,
    file.replace('github.com', 'github.com.evil.test'), file.replace('RusTor/releases', 'Other/releases'),
    `${file}?token=x`, 'javascript:alert(1)']) expect(downloadDetails(url, 'hero')).toBeNull();
});

it('offers equally available accept and decline controls and lets visitors reopen preferences', async () => {
  render(<AnalyticsConsent />);
  fireEvent.click(await screen.findByRole('button', { name: 'Decline analytics' }));
  expect(screen.queryByRole('button', { name: 'Allow analytics' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Analytics preferences' }));
  expect(screen.getByRole('button', { name: 'Allow analytics' })).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Keep current choice' }));
  expect(screen.getByRole('button', { name: 'Analytics preferences' }).getAttribute('aria-expanded')).toBe('false');
});

it('wires hero, package and archive clicks independently of arrow animation or motion preference', () => {
  const tracking = vi.spyOn(analytics(), 'download');
  vi.stubGlobal('matchMedia', () => ({ matches: true }));
  const view = render(<><DownloadLink id="hero-download" href={file} label="Hero download" />
    <DownloadLink href={file} label="Package download" />
    <ReleaseArchive releases={[{ id: 1, name: 'Release', published: '2026-11-05T00:00:00Z',
      url: 'https://github.com/adityapandeydev/RusTor/releases/tag/v0.1.2', assets: [{ url: file, name: 'Installer', size: 100 }] }]} /></>);
  // Cancel navigation in the test only; production keeps the native anchor action.
  view.container.addEventListener('click', event => event.preventDefault());
  fireEvent.click(screen.getByRole('link', { name: 'Hero download' }));
  fireEvent.click(screen.getByRole('link', { name: 'Package download' }));
  fireEvent.click(screen.getByRole('button', { name: /Release/ }));
  fireEvent.click(screen.getByRole('link', { name: /Installer/ }));
  expect(tracking.mock.calls).toEqual([[file, 'hero'], [file, 'platform-picker'], [file, 'release-archive']]);
  vi.unstubAllGlobals();
});
