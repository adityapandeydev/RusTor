import { useEffect, useState } from 'react';
import { packageLinks, preferredRelease, fileSize, REPOSITORY } from '../releases.mjs';
import { DownloadLink, ReleaseArchive } from './Downloads.jsx';
import { useSiteMotion } from './motion.js';
import AnalyticsConsent from './AnalyticsConsent.jsx';

const descriptions = {
  windows: 'Windows x64. The installer can set up WebView2 if needed.',
  linux: 'Linux x64. Pick AppImage, or a Debian package for Debian and Ubuntu.',
  'mac-arm': 'For Macs with an M series chip. Check About This Mac if you’re unsure.',
  'mac-intel': 'For Macs with an Intel processor. Check About This Mac if you’re unsure.',
};

export default function App({ initialReleases = [] }) {
  const [releases, setReleases] = useState(initialReleases);
  const [status, setStatus] = useState(initialReleases.length ? 'ready' : 'loading');
  const [platform, setPlatform] = useState('windows');
  const [architecture, setArchitecture] = useState('mac-arm');
  const [theme, setTheme] = useState('dark');
  const [year, setYear] = useState(2026);
  const [previewAlt, setPreviewAlt] = useState('RusTor Downloads in dark mode showing illustrative downloading, seeding and paused transfers');
  const { reduced, replayAssembly } = useSiteMotion();
  const release = preferredRelease(releases);
  const packages = packageLinks(release, platform === 'mac' ? architecture : platform);
  const platformDescription = descriptions[platform === 'mac' ? architecture : platform];

  useEffect(() => {
    let active = true;
    setPlatform(/Mac/i.test(navigator.userAgent) ? 'mac' : /Linux/i.test(navigator.userAgent) && !/Android/i.test(navigator.userAgent) ? 'linux' : 'windows');
    setYear(new Date().getFullYear());
    async function loadReleases() {
      try {
        const response = await fetch('./releases.json', { signal: AbortSignal.timeout(12000) });
        if (!response.ok) throw new Error('Release data unavailable');
        const data = await response.json();
        if (!Array.isArray(data.releases)) throw new Error('Invalid release data');
        if (active) { setReleases(data.releases); setStatus(data.releases.length ? 'ready' : 'empty'); }
      } catch { if (active) setStatus('failed'); }
    }
    void loadReleases();
    return () => { active = false; };
  }, []);

  function changeTheme(next) {
    setTheme(next);
    setPreviewAlt(`RusTor Downloads in ${next} mode, with example transfers`);
    if (!matchMedia('(prefers-reduced-motion: reduce)').matches)
      document.querySelector('#hero-screen').animate([{ opacity: .55 }, { opacity: 1 }], { duration: 220, easing: 'cubic-bezier(.23,1,.32,1)' });
  }

  return <>
<a className="skip-link" href="#main">Skip to content</a>
  <header className="site-header frame">
    <a className="wordmark" href="./" aria-label="RusTor home"><img src="./assets/mark.svg" width="40" height="40" alt="" />RusTor</a>
    <nav aria-label="Main navigation"><a href="#experience">The app</a><a href="#releases">Releases</a><a className="source-link" href="https://github.com/adityapandeydev/RusTor">Source code <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 18 18 6M6 6h12v12"/></svg></a></nav>
  </header>
  <main id="main">
    <section className="hero frame" aria-labelledby="hero-title">
      <div className="hero-copy relative z-[1]">
        <h1 id="hero-title" className="heading-1">Good things.<br />Piece by piece.</h1>
        <p className="hero-description body-1">A torrent client built with Rust. Search, choose your files, and keep every transfer in view.</p>
        <DownloadLink id="hero-download" href={packages[0]?.url ?? '#downloads'} label={packages[0] ? `Download for ${platform === 'mac' ? 'macOS' : platform === 'linux' ? 'Linux' : 'Windows'}` : 'Download RusTor'} />
        <p className="caption hero-proof">Free &amp; open source.<span>Windows · Linux · macOS</span></p>
      </div>
      <div className="transfer-art" aria-hidden="true">
        <svg className="assembly" viewBox="0 0 480 480" fill="none">
          <path className="route" d="M32 72h80c48 0 48 112 96 112h32M448 112h-72c-48 0-48 72-88 72h-48M16 360h80c56 0 56-72 104-72h40M464 384h-72c-48 0-48-96-104-96h-48"/>
          <g className="arrow-tiles" fill="currentColor">
            <rect x="205" y="96" width="30" height="30" style={{'--x': '-130px', '--y': '-90px', '--i': '0'}}/><rect x="245" y="96" width="30" height="30" style={{'--x': '150px', '--y': '-70px', '--i': '1'}}/>
            <rect x="205" y="136" width="30" height="30" style={{'--x': '-140px', '--y': '-10px', '--i': '2'}}/><rect x="245" y="136" width="30" height="30" style={{'--x': '120px', '--y': '-40px', '--i': '3'}}/>
            <rect x="205" y="176" width="30" height="30" style={{'--x': '-150px', '--y': '60px', '--i': '4'}}/><rect x="245" y="176" width="30" height="30" style={{'--x': '150px', '--y': '30px', '--i': '5'}}/>
            <rect x="125" y="216" width="30" height="30" style={{'--x': '-70px', '--y': '100px', '--i': '6'}}/><rect x="165" y="216" width="30" height="30" style={{'--x': '-90px', '--y': '-150px', '--i': '7'}}/>
            <rect x="205" y="216" width="30" height="30" style={{'--x': '-90px', '--y': '100px', '--i': '8'}}/><rect x="245" y="216" width="30" height="30" style={{'--x': '100px', '--y': '-140px', '--i': '9'}}/>
            <rect x="285" y="216" width="30" height="30" style={{'--x': '70px', '--y': '100px', '--i': '10'}}/><rect x="325" y="216" width="30" height="30" style={{'--x': '80px', '--y': '50px', '--i': '11'}}/>
            <rect x="165" y="256" width="30" height="30" style={{'--x': '-100px', '--y': '80px', '--i': '12'}}/><rect x="205" y="256" width="30" height="30" style={{'--x': '-90px', '--y': '120px', '--i': '13'}}/>
            <rect x="245" y="256" width="30" height="30" style={{'--x': '110px', '--y': '70px', '--i': '14'}}/><rect x="285" y="256" width="30" height="30" style={{'--x': '90px', '--y': '100px', '--i': '15'}}/>
            <rect x="205" y="296" width="30" height="30" style={{'--x': '-100px', '--y': '70px', '--i': '16'}}/><rect x="245" y="296" width="30" height="30" style={{'--x': '120px', '--y': '80px', '--i': '17'}}/>
            <rect x="125" y="376" width="30" height="10" style={{'--x': '-90px', '--y': '20px', '--i': '18'}}/><rect x="165" y="376" width="30" height="10" style={{'--x': '-90px', '--y': '30px', '--i': '19'}}/>
            <rect x="205" y="376" width="30" height="10" style={{'--x': '-80px', '--y': '30px', '--i': '20'}}/><rect x="245" y="376" width="30" height="10" style={{'--x': '80px', '--y': '30px', '--i': '21'}}/>
            <rect x="285" y="376" width="30" height="10" style={{'--x': '90px', '--y': '30px', '--i': '22'}}/><rect x="325" y="376" width="30" height="10" style={{'--x': '90px', '--y': '20px', '--i': '23'}}/>
          </g>
          <g className="loose-tiles" fill="currentColor"><rect x="64" y="56" width="10" height="10"/><rect x="401" y="88" width="10" height="10"/><rect x="48" y="339" width="10" height="10"/><rect x="416" y="360" width="10" height="10"/></g>
        </svg>
      </div>
      <div className="hero-bottom"><a href="#experience">Take a closer look <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4v16m-6-6 6 6 6-6"/></svg></a><button type="button" className="motion-control" id="replay-motion" disabled={reduced} onClick={replayAssembly} aria-label={reduced ? 'Animation disabled by your reduced motion preference' : 'Replay the piece assembly animation'}>{reduced ? 'Motion reduced ' : 'Replay animation '}<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 10a8 8 0 1 1 1 8M4 4v6h6"/></svg></button></div>
    </section>
    <section className="showcase pearl" id="experience" aria-labelledby="experience-title">
      <div className="frame section-space">
        <div className="section-heading"><h2 className="heading-2" id="experience-title" data-reveal="">Everything in motion.<br />In one place.</h2><div className="showcase-intro"><p className="body-1">A clear view of what’s downloading, what’s seeding, and what’s next.</p><div className="theme-switch" role="group" aria-label="Preview app appearance"><button type="button" data-theme="dark" aria-pressed={theme === 'dark'} onClick={() => changeTheme('dark')}>Dark</button><button type="button" data-theme="light" aria-pressed={theme === 'light'} onClick={() => changeTheme('light')}>Light</button></div></div></div>
        <figure className="product-view" data-reveal=""><img id="hero-screen" src={theme === 'light' ? './assets/light.jpg' : './assets/downloads.jpg'} alt={previewAlt} width="1240" height="780" loading="lazy" /><figcaption className="caption">The real RusTor interface. Example transfers shown.</figcaption></figure>
      </div>
    </section>
    <section className="control-section pearl" aria-labelledby="control-title">
      <div className="frame section-space">
        <div className="control-statement"><h2 id="control-title" className="heading-1"><span data-word="">Your</span> <span data-word="">files.</span><br /><span data-word="">Your</span> <span data-word="">call.</span></h2><p className="body-1">Choose what comes in.<br />Decide how it gets there.</p></div>
        <article className="feature-row"><div className="feature-copy" data-reveal=""><h3 className="heading-3">Leave the rest behind.</h3><p>Open a magnet or torrent and choose the files you actually want. Pick a folder before your download begins.</p><p className="feature-aside caption">A whole torrent. Or just your part of it.</p></div><figure className="feature-image" data-reveal=""><img src="./assets/files.jpg" alt="RusTor file selection panel with individual files, sizes, folder selection and the Start download button" width="942" height="619" loading="lazy" /></figure></article>
        <article className="feature-row feature-reverse"><div className="feature-copy" data-reveal=""><h3 className="heading-3">You set the pace.</h3><p>Limit upload and download speeds. Pause, resume, or stop a torrent while keeping your files. Reopen the app and pick up your saved session.</p><p className="feature-aside caption">Your bandwidth. Your schedule.</p></div><figure className="feature-image" data-reveal=""><img src="./assets/settings.jpg" alt="RusTor download settings with a default folder and upload and download speed limits" width="952" height="396" loading="lazy" /></figure></article>
        <div className="discover-note"><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="m16 8-3 5-5 3 3-5Z"/></svg><p>Start in Discover. Search Movies, TV, Games and Anime, recognize titles with cover artwork where available, then bring your selection straight into Downloads.</p></div>
      </div>
    </section>
    <section className="downloads" id="downloads" aria-labelledby="downloads-title"><div className="frame section-space download-layout">
      <div className="download-intro"><h2 className="heading-2" id="downloads-title" data-reveal="">A new home for<br />your downloads.</h2><p className="body-1">Pick your system.<br />We’ll take it from here.</p><p className="caption release-label" id="current-release" aria-live="polite">{release ? `${release.tag} · ${release.preview ? 'Preview release' : 'Release'}` : 'See the available version on GitHub.'}</p><a className="text-link caption" href="#releases">Looking for an older version?</a></div>
      <div className="download-picker" data-reveal="">
        <div className="platform-picker" role="group" aria-label="Choose your operating system"><button type="button" data-platform="windows" aria-pressed={platform === 'windows'} onClick={() => setPlatform('windows')}>Windows</button><button type="button" data-platform="linux" aria-pressed={platform === 'linux'} onClick={() => setPlatform('linux')}>Linux</button><button type="button" data-platform="mac" aria-pressed={platform === 'mac'} onClick={() => setPlatform('mac')}>macOS</button></div>
        <div className="architecture-picker" role="group" aria-label="Choose your Mac processor" hidden={platform !== 'mac'}><button type="button" data-architecture="mac-arm" aria-pressed={architecture === 'mac-arm'} onClick={() => setArchitecture('mac-arm')}>Apple Silicon</button><button type="button" data-architecture="mac-intel" aria-pressed={architecture === 'mac-intel'} onClick={() => setArchitecture('mac-intel')}>Intel</button></div>
        <p id="platform-description">{platformDescription}</p>
        <div id="package-links" className="package-links" aria-live="polite">
          {!release ? <DownloadLink href={`${REPOSITORY}/releases`} label="Choose a release on GitHub" external /> : packages.length === 0 ? <DownloadLink href={release.url} label="View this release on GitHub" /> : packages.map((asset, index) => (
            <div key={asset.url} className={`package-option${index ? ' secondary' : ''}`}>
              <DownloadLink href={asset.url} label={`Download ${asset.label}`} /><span className="caption">{fileSize(asset.size)}</span>
            </div>
          ))}
        </div>
        <p className="caption picker-note">Preview packages are unsigned. Windows and macOS may show a security warning. Linux and macOS desktop checks are still in progress.</p><p className="caption" id="download-status" role="status">{status === 'failed' ? <>The release list couldn’t load. <a href={`${REPOSITORY}/releases`}>Open GitHub Releases instead.</a></> : status === 'empty' ? 'No public packages yet. Check GitHub Releases for updates.' : null}</p><noscript><p>Get all installers from <a href="https://github.com/adityapandeydev/RusTor/releases">GitHub Releases</a>.</p></noscript>
      </div>
    </div></section>
    <section className="releases pearl" id="releases" aria-labelledby="releases-title"><div className="frame section-space archive-layout"><div className="archive-intro"><h2 className="heading-2" id="releases-title" data-reveal="">Recent<br />releases.</h2><p>What’s new and what changed in the latest three releases. Find every older version on GitHub.</p><a className="text-link caption" href="https://github.com/adityapandeydev/RusTor/releases">View on GitHub</a></div><ReleaseArchive releases={releases} /></div></section>
    <section className="questions pearl" aria-labelledby="questions-title"><div className="frame section-space faq-layout"><h2 className="heading-2" id="questions-title" data-reveal="">Before you<br />get going.</h2><div className="faq-grid">
      <details><summary>Is RusTor free?</summary><p>Yes. RusTor’s original code is open source under the MIT License. You can use it, read the code, and contribute on GitHub.</p></details>
      <details><summary>What systems can I use?</summary><p>Preview packages are available for Windows x64, Linux x64, and Macs with Apple Silicon or Intel processors. Linux and macOS installation and desktop behavior are not yet fully validated.</p></details>
      <details><summary>What happens when I stop a torrent?</summary><p>The × control stops downloading or seeding and removes the torrent from your saved session. Complete and partial files remain on disk.</p></details>
      <details><summary>Will my downloads survive a restart?</summary><p>RusTor saves your torrents, destinations and file selections. When you reopen it, it restores the session and checks existing data before continuing. Large downloads may take a while to recheck.</p></details>
      <details><summary>Does it run when I close the app?</summary><p>In 0.2.0 and later, enable Continue running when closed in Download settings to keep transfers running in the tray. Use Quit RusTor to exit. Linux keeps a minimized window as a fallback; sleep, logout and shutdown still interrupt transfers. You can also choose to start RusTor at login.</p></details>
      <details><summary>How do updates work?</summary><p>From 0.2.0, installed Windows apps, macOS apps and Linux AppImages can check for updates in Download settings. Update packages are signature-verified before you choose Install and restart. Versions 0.1.0–0.1.2 need one manual install-over upgrade; standalone Windows ZIP and Debian packages use manual upgrades.</p></details>
      <details><summary>Why might my computer show a warning?</summary><p>Windows publisher signing and macOS notarization are not configured. Updater signatures verify update packages but do not remove operating-system installation warnings. Check the release notes and SHA256SUMS before installing. Download only from this project’s GitHub releases.</p></details>
      <details><summary>Does RusTor hide my IP address?</summary><p>No. BitTorrent peers can see your IP address. RusTor does not include a VPN or anonymous routing. Share only content you have permission to distribute.</p></details>
      <details><summary>Where does the search come from?</summary><p>RusTor’s search behavior draws on Torlink. Results depend on external source availability; metadata can be missing or up to five minutes old. Torrents using only BitTorrent v2 are currently unsupported.</p></details>
      <details><summary>Where does cover artwork come from?</summary><p>From 0.2.1, RusTor uses artwork supplied by search providers first. When a cover is missing, it can match a cleaned title and year against IMDb for movies or TVmaze for TV and anime. Lookups send the cleaned title, never magnets, local file paths or download history. Covers are cached locally and can appear in Downloads after a restart. Missing or ambiguous matches keep a file placeholder. TVmaze metadata is used under CC BY-SA.</p></details>
    </div></div></section>
  </main>
  <footer className="site-footer"><div className="frame"><div className="footer-top"><a className="footer-name" href="#main">RusTor<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 18 18 6M6 6h12v12"/></svg></a><p>Made by <a href="https://github.com/adityapandeydev">Aditya Pandey</a>.<br />Piece by piece.</p></div><div className="footer-bottom"><span>© <span id="year">{year}</span> RusTor</span><nav aria-label="Footer navigation"><a href="https://github.com/adityapandeydev/RusTor">Source code</a><a href="https://github.com/adityapandeydev/RusTor/issues">Report a bug</a><a href="https://github.com/adityapandeydev/RusTor/blob/main/LICENSE">MIT License</a><a href="https://github.com/baairon/torlink">Torlink</a></nav></div><p className="caption footer-privacy">Optional Google Analytics measures site visits and download clicks only with your permission. GitHub hosts the site and downloads. Product views show illustrative transfers.</p><AnalyticsConsent /></div></footer>
  </>;
}
