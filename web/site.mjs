import { packageLinks, preferredRelease, fileSize, REPOSITORY } from './releases.mjs';

const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
const $ = selector => document.querySelector(selector);
let releases = [];
let shown = 0;
let platform = /Mac/i.test(navigator.userAgent) ? 'mac' : /Linux/i.test(navigator.userAgent) && !/Android/i.test(navigator.userAgent) ? 'linux' : 'windows';
let architecture = 'mac-arm';

function node(tag, text, className) {
  const element = document.createElement(tag);
  if (text) element.textContent = text;
  if (className) element.className = className;
  return element;
}

function link(text, url, className) {
  const element = node('a', text, className);
  element.href = url;
  return element;
}


function downloadLink(text, url) {
  const anchor = link(null, url, 'button download-button');
  const icon = node('span', null, 'button-icon');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('d', 'M12 4v14m-6-6 6 6 6-6M5 21h14');
  svg.append(path);
  icon.append(svg);
  anchor.append(node('span', text), icon);
  return anchor;
}

function drawDownloads() {
  for (const button of document.querySelectorAll('[data-platform]')) button.setAttribute('aria-pressed', String(button.dataset.platform === platform));
  for (const button of document.querySelectorAll('[data-architecture]')) button.setAttribute('aria-pressed', String(button.dataset.architecture === architecture));
  $('.architecture-picker').hidden = platform !== 'mac';
  const descriptions = {
    windows: 'Windows x64. The installer can set up WebView2 if needed.',
    linux: 'Linux x64. Pick AppImage, or a Debian package for Debian and Ubuntu.',
    'mac-arm': 'For Macs with an M series chip. Check About This Mac if you’re unsure.',
    'mac-intel': 'For Macs with an Intel processor. Check About This Mac if you’re unsure.',
  };
  const selected = platform === 'mac' ? architecture : platform;
  $('#platform-description').textContent = descriptions[selected];
  const release = preferredRelease(releases);
  const packages = packageLinks(release, selected);
  if (!release) return;
  const container = $('#package-links');
  container.replaceChildren();
  if (!packages.length) container.append(downloadLink('View this release on GitHub', release.url));
  packages.forEach((asset, index) => {
    const option = node('div', null, `package-option${index ? ' secondary' : ''}`);
    option.append(downloadLink(`Download ${asset.label}`, asset.url), node('span', fileSize(asset.size), 'caption'));
    container.append(option);
  });
  const primary = packages[0];
  $('#hero-download').href = primary?.url ?? '#downloads';
  $('[data-download-label]').textContent = primary ? `Download for ${platform === 'mac' ? 'macOS' : platform === 'linux' ? 'Linux' : 'Windows'}` : 'Download RusTor';
  $('#current-release').textContent = `${release.tag} · ${release.preview ? 'Preview release' : 'Release'}`;
}

function renderMore() {
  const container = $('#release-list');
  if (shown === 0) container.replaceChildren();
  for (const release of releases.slice(shown, shown + 5)) {
    const entry = node('article', null, 'release-entry');
    const summary = node('button', null, 'release-toggle');
    summary.type = 'button';
    summary.setAttribute('aria-expanded', 'false');
    summary.setAttribute('aria-controls', `assets-${release.id}`);
    const title = node('span', release.name, 'release-title');
    title.append(node('span', release.preview ? 'Preview' : 'Release', 'release-kind'));
    const meta = node('span', null, 'release-meta');
    const time = node('time', new Intl.DateTimeFormat('en', { day: 'numeric', month: 'short', year: 'numeric', timeZone: 'UTC' }).format(new Date(release.published)));
    time.dateTime = release.published;
    const icon = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    icon.setAttribute('viewBox', '0 0 24 24');
    icon.setAttribute('aria-hidden', 'true');
    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    path.setAttribute('d', 'M12 5v14M5 12h14');
    icon.append(path);
    meta.append(time, icon);
    summary.append(title, meta);
    const assets = node('div', null, 'release-assets');
    for (const asset of release.assets) assets.append(link(`${asset.name} · ${fileSize(asset.size)}`, asset.url));
    const panel = node('div', null, 'release-panel');
    panel.id = `assets-${release.id}`;
    panel.hidden = true;
    panel.append(assets, link('Read release notes on GitHub', release.url, 'release-notes-link'));
    summary.addEventListener('click', () => {
      panel.hidden = !panel.hidden;
      summary.setAttribute('aria-expanded', String(!panel.hidden));
    });
    entry.append(summary, panel);
    container.append(entry);
  }
  shown = Math.min(shown + 5, releases.length);
  $('#more-releases').hidden = shown >= releases.length;
}

// Delegate so package links rebuilt by the platform picker get the same feedback.
// Keep the anchor's default action: the download never waits for the animation.
document.addEventListener('click', event => {
  const arrow = event.target.closest('.download-button')?.querySelector('.button-icon svg');
  if (!arrow || reducedMotion.matches) return;
  arrow.getAnimations().forEach(animation => animation.cancel());
  arrow.animate([{ transform: 'rotateX(0deg)' }, { transform: 'rotateX(-360deg)' }],
    { duration: 700, easing: 'cubic-bezier(.25,.1,.25,1)' });
});

for (const button of document.querySelectorAll('[data-platform]')) button.addEventListener('click', () => { platform = button.dataset.platform; drawDownloads(); });
for (const button of document.querySelectorAll('[data-architecture]')) button.addEventListener('click', () => { architecture = button.dataset.architecture; drawDownloads(); });
$('#more-releases').addEventListener('click', renderMore);
for (const button of document.querySelectorAll('[data-theme]')) button.addEventListener('click', () => {
  const light = button.dataset.theme === 'light';
  const image = $('#hero-screen');
  image.src = light ? './assets/light.jpg' : './assets/downloads.jpg';
  image.alt = `RusTor Downloads in ${light ? 'light' : 'dark'} mode, with example transfers`;
  for (const item of document.querySelectorAll('[data-theme]')) item.setAttribute('aria-pressed', String(item === button));
  if (!reducedMotion.matches) image.animate([{ opacity: .55 }, { opacity: 1 }], { duration: 220, easing: 'cubic-bezier(.23,1,.32,1)' });
});
$('#year').textContent = String(new Date().getFullYear());
drawDownloads();

const assembly = $('.assembly');
const replay = $('#replay-motion');
function playAssembly() {
  assembly.classList.remove('is-playing');
  if (reducedMotion.matches) return;
  // Restart a finite CSS sequence. No background loop or scroll handler.
  void assembly.getBoundingClientRect();
  assembly.classList.add('is-playing');
}
function updateMotionPreference() {
  document.getAnimations().forEach(animation => animation.cancel());
  replay.disabled = reducedMotion.matches;
  replay.setAttribute('aria-label', reducedMotion.matches ? 'Animation disabled by your reduced motion preference' : 'Replay the piece assembly animation');
  replay.firstChild.textContent = reducedMotion.matches ? 'Motion reduced ' : 'Replay animation ';
  playAssembly();
}
replay.addEventListener('click', playAssembly);
reducedMotion.addEventListener('change', updateMotionPreference);
updateMotionPreference();

if ('IntersectionObserver' in window) {
  const reveal = new IntersectionObserver(entries => {
    for (const entry of entries) if (entry.isIntersecting) {
      if (!reducedMotion.matches) entry.target.animate([
        { opacity: .4, transform: 'translateY(24px)' },
        { opacity: 1, transform: 'translateY(0)' },
      ], { duration: 650, easing: 'cubic-bezier(.22,1,.36,1)' });
      reveal.unobserve(entry.target);
    }
  }, { threshold: .12 });
  document.querySelectorAll('[data-reveal]').forEach(element => reveal.observe(element));
  const words = [...document.querySelectorAll('[data-word]')];
  const statement = new IntersectionObserver(entries => {
    if (!entries.some(entry => entry.isIntersecting)) return;
    if (!reducedMotion.matches) words.forEach((word, index) => word.animate([
      { opacity: .35 }, { opacity: 1 },
    ], { duration: 650, delay: index * 140, easing: 'cubic-bezier(.22,1,.36,1)' }));
    statement.disconnect();
  }, { threshold: .5 });
  statement.observe($('.control-statement'));
}

try {
  const response = await fetch('./releases.json', { signal: AbortSignal.timeout(12000) });
  if (!response.ok) throw new Error('Release data unavailable');
  const data = await response.json();
  if (!Array.isArray(data.releases)) throw new Error('Invalid release data');
  releases = data.releases;
  if (releases.length) { drawDownloads(); renderMore(); }
  else $('#download-status').textContent = 'No public packages yet. Check GitHub Releases for updates.';
} catch {
  $('#download-status').replaceChildren(document.createTextNode('The release list couldn’t load. '), link('Open GitHub Releases instead.', `${REPOSITORY}/releases`));
}
