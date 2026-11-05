import { useState } from 'react';
import { fileSize, REPOSITORY } from '../releases.mjs';
import { analytics } from './analytics.js';

export function DownloadLink({ href, label, id, external = false }) {
  function animateArrow(event) {
    analytics().download(href, id === 'hero-download' ? 'hero' : 'platform-picker');
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const arrow = event.currentTarget.querySelector('.button-icon svg');
    arrow.getAnimations().forEach(animation => animation.cancel());
    arrow.animate([{ transform: 'rotateX(0deg)' }, { transform: 'rotateX(-360deg)' }],
      { duration: 700, easing: 'cubic-bezier(.25,.1,.25,1)' });
  }
  return <a className="button download-button" id={id} href={href} onClick={animateArrow}>
    <span data-download-label={id === 'hero-download' ? '' : undefined}>{label}</span>
    <span className="button-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d={external ? 'M6 18 18 6M6 6h12v12' : 'M12 4v14m-6-6 6 6 6-6M5 21h14'} /></svg></span>
  </a>;
}

function ReleaseEntry({ release }) {
  const [expanded, setExpanded] = useState(false);
  return <article className="release-entry">
    <button type="button" className="release-toggle" aria-expanded={expanded} aria-controls={`assets-${release.id}`} onClick={() => setExpanded(!expanded)}>
      <span className="release-title">{release.name}<span className="release-kind">{release.preview ? 'Preview' : 'Release'}</span></span>
      <span className="release-meta"><time dateTime={release.published}>{new Intl.DateTimeFormat('en', { day: 'numeric', month: 'short', year: 'numeric', timeZone: 'UTC' }).format(new Date(release.published))}</time><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg></span>
    </button>
    <div className="release-panel" id={`assets-${release.id}`} hidden={!expanded}>
      <div className="release-assets">{release.assets.map(asset => <a key={asset.url} href={asset.url} onClick={() => analytics().download(asset.url, 'release-archive')}>{asset.name} · {fileSize(asset.size)}</a>)}</div>
      <a className="release-notes-link" href={release.url}>Read release notes on GitHub</a>
    </div>
  </article>;
}

export function ReleaseArchive({ releases }) {
  const [shown, setShown] = useState(5);
  return <div className="archive-content min-w-0">
    <div id="release-list" className="release-list" aria-live="polite">
      {releases.length ? releases.slice(0, shown).map(release => <ReleaseEntry key={release.id} release={release} />) : <p><a href={`${REPOSITORY}/releases`}>Browse all releases on GitHub</a></p>}
    </div>
    <button className="button secondary-button more-releases" type="button" id="more-releases" hidden={shown >= releases.length} onClick={() => setShown(shown + 5)}>Show older releases</button>
  </div>;
}
