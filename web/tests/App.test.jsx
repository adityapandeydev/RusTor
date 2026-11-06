import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { renderToString } from 'react-dom/server';
import { hydrateRoot } from 'react-dom/client';
import App from '../src/App.jsx';
import { normalizeReleases } from '../releases.mjs';

const repository = 'https://github.com/adityapandeydev/RusTor';
const names = ['RusTor_0.1.2_x64-setup.exe', 'RusTor_0.1.2_x64-standalone.zip', 'RusTor_0.1.2_amd64.AppImage', 'RusTor_0.1.2_amd64.deb', 'RusTor_0.1.2_aarch64.dmg', 'RusTor_0.1.2_x64.dmg'];
const release = normalizeReleases([{ id: 1, name: 'RusTor 0.1.2', tag_name: 'v0.1.2', draft: false, prerelease: false, published_at: '2026-11-05T05:18:59Z', html_url: `${repository}/releases/tag/v0.1.2`, assets: names.map(name => ({ name, state: 'uploaded', size: 100, browser_download_url: `${repository}/releases/download/v0.1.2/${name}` })) }])[0];
let preference;
let listeners;
let animations;

beforeEach(() => {
  listeners = new Set();
  animations = [];
  preference = { matches: false, addEventListener: (_, listener) => listeners.add(listener), removeEventListener: (_, listener) => listeners.delete(listener) };
  vi.stubGlobal('matchMedia', () => preference);
  vi.stubGlobal('IntersectionObserver', class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, json: async () => ({ releases: [release] }) })));
  document.getAnimations = () => animations.filter(animation => !animation.cancelled);
  Element.prototype.getAnimations = function () { return animations.filter(animation => animation.element === this && !animation.cancelled); };
  Element.prototype.animate = vi.fn(function (frames, options) { const animation = { element: this, frames, options, cancelled: false, cancel() { this.cancelled = true; } }; animations.push(animation); return animation; });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it('preserves all platform downloads, hero links, previews and archive controls', async () => {
  render(<App />);
  fireEvent.click(screen.getByRole('button', { name: 'Windows', exact: true }));
  await screen.findByRole('link', { name: 'Download Windows installer' });
  const hero = () => document.querySelector('#hero-download');
  expect(hero().href).toContain('x64-setup.exe');
  fireEvent.click(screen.getByRole('button', { name: 'Linux', exact: true }));
  expect(screen.getByRole('link', { name: 'Download AppImage' }).href).toContain('amd64.AppImage');
  expect(screen.getByRole('link', { name: 'Download Debian package' }).href).toContain('amd64.deb');
  expect(hero().href).toContain('amd64.AppImage');
  fireEvent.click(screen.getByRole('button', { name: 'macOS' }));
  expect(screen.getByRole('link', { name: 'Download Apple Silicon DMG' }).href).toContain('aarch64.dmg');
  fireEvent.click(screen.getByRole('button', { name: 'Intel', exact: true }));
  expect(screen.getByRole('link', { name: 'Download Intel DMG' }).href).toContain('x64.dmg');
  expect(hero().href).toContain('x64.dmg');
  fireEvent.click(screen.getByRole('button', { name: 'Light', exact: true }));
  expect(document.querySelector('#hero-screen').getAttribute('src')).toBe('./assets/light.jpg');
  expect(screen.getByRole('button', { name: 'Light', exact: true }).getAttribute('aria-pressed')).toBe('true');
  const toggle = screen.getByRole('button', { name: /RusTor 0.1.2\s*Release/ });
  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(document.querySelectorAll('.release-assets a')).toHaveLength(6);
  fireEvent.click(toggle);
  expect(document.querySelector('.release-panel').hidden).toBe(true);
});

it('keeps fallback download links for offline, invalid and empty feeds', async () => {
  for (const data of [new Error('offline'), { releases: null }, { releases: [] }]) {
    fetch.mockImplementationOnce(async () => { if (data instanceof Error) throw data; return { ok: true, json: async () => data }; });
    const view = render(<App />);
    await waitFor(() => expect(screen.getByRole('status').textContent).toMatch(/couldn’t load|No public packages/));
    expect(screen.getByRole('link', { name: 'Choose a release on GitHub' }).href).toBe(`${repository}/releases`);
    expect(document.querySelector('#hero-download').getAttribute('href')).toBe('#downloads');
    view.unmount();
  }
});

it('shows only the latest three feed entries and renders release titles as text', async () => {
  fetch.mockResolvedValueOnce({ ok: true, json: async () => ({ releases: Array.from({ length: 12 }, (_, id) => ({ ...release, id, name: '<img src=x onerror=alert(1)>' })) }) });
  render(<App />);
  await waitFor(() => expect(document.querySelectorAll('.release-entry')).toHaveLength(3));
  expect([...document.querySelectorAll('.release-toggle')].map(button => button.getAttribute('aria-controls'))).toEqual(['assets-0', 'assets-1', 'assets-2']);
  expect(screen.queryByRole('button', { name: 'Show older releases' })).toBeNull();
  expect(document.querySelectorAll('.release-entry img')).toHaveLength(0);
});

it('loops arrows once without blocking navigation and honors live reduced motion changes', async () => {
  const view = render(<App />);
  fireEvent.click(screen.getByRole('button', { name: 'Windows', exact: true }));
  await screen.findByRole('link', { name: 'Download Windows installer' });
  const link = document.querySelector('#hero-download');
  // jsdom cannot navigate; platform tests above verify the real package URLs.
  link.removeAttribute('href');
  const click = () => { const event = new MouseEvent('click', { bubbles: true, cancelable: true }); fireEvent(link.querySelector('svg'), event); expect(event.defaultPrevented).toBe(false); return animations.at(-1); };
  const first = click();
  expect(first.frames.map(frame => frame.transform)).toEqual(['rotateX(0deg)', 'rotateX(-360deg)']);
  expect(first.options.duration).toBe(700);
  click();
  expect(first.cancelled).toBe(true);
  act(() => { preference.matches = true; listeners.forEach(listener => listener()); });
  expect(document.querySelector('.assembly').classList.contains('is-playing')).toBe(false);
  expect(screen.getByRole('button', { name: /Animation disabled/ }).disabled).toBe(true);
  const count = animations.length;
  fireEvent.click(link);
  expect(animations).toHaveLength(count);
  fireEvent.click(screen.getByRole('button', { name: 'Light', exact: true }));
  expect(animations).toHaveLength(count);
  act(() => { preference.matches = false; listeners.forEach(listener => listener()); });
  expect(document.querySelector('.assembly').classList.contains('is-playing')).toBe(true);
  view.unmount();
  expect(listeners.size).toBe(0);
});

it('pre-renders the complete page and hydrates without replacing it or reporting mismatches', async () => {
  const container = document.createElement('div');
  container.innerHTML = renderToString(<App />);
  document.body.append(container);
  const heading = container.querySelector('h1');
  expect(heading.textContent).toBe('Good things.Piece by piece.');
  expect(container.querySelectorAll('.faq-grid details')).toHaveLength(9);
  expect(container.querySelector('.faq-grid').textContent).toContain('How do updates work?');
  expect(container.querySelector('#package-links a').href).toBe(`${repository}/releases`);
  const onRecoverableError = vi.fn();
  let root;
  await act(async () => { root = hydrateRoot(container, <App />, { onRecoverableError }); });
  await waitFor(() => expect(container.querySelector('#current-release').textContent).toBe('v0.1.2 · Release'));
  expect(container.querySelector('h1')).toBe(heading);
  expect(onRecoverableError).not.toHaveBeenCalled();
  await act(async () => root.unmount());
  container.remove();
});

it('pre-renders release downloads and hydrates the same snapshot without replacing the page', async () => {
  const container = document.createElement('div');
  container.innerHTML = renderToString(<App initialReleases={[release]} />);
  document.body.append(container);
  const heading = container.querySelector('h1');
  expect(container.querySelector('#package-links a').href).toContain('x64-setup.exe');
  expect(container.querySelector('#release-list').textContent).toContain('RusTor 0.1.2');
  const onRecoverableError = vi.fn();
  let root;
  await act(async () => { root = hydrateRoot(container, <App initialReleases={[release]} />, { onRecoverableError }); });
  expect(container.querySelector('h1')).toBe(heading);
  expect(onRecoverableError).not.toHaveBeenCalled();
  await act(async () => root.unmount());
  container.remove();
});

it.each([
  ['Mozilla/5.0 (Windows NT 10.0; Win64; x64)', 'windows', 'x64-setup.exe'],
  ['Mozilla/5.0 (X11; Linux x86_64)', 'linux', 'amd64.AppImage'],
  ['Mozilla/5.0 (Macintosh; Intel Mac OS X)', 'mac', 'aarch64.dmg'],
  ['Mozilla/5.0 (Linux; Android 15)', 'windows', 'x64-setup.exe'],
])('selects the existing platform default for %s', async (userAgent, platform, filename) => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(userAgent);
  render(<App />);
  await waitFor(() => expect(document.querySelector('#hero-download').href).toContain(filename));
  expect(document.querySelector(`[data-platform="${platform}"]`).getAttribute('aria-pressed')).toBe('true');
});
