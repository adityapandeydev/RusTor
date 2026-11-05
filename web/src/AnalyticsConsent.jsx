import { useEffect, useState } from 'react';
import { analytics } from './analytics.js';

export default function AnalyticsConsent() {
  const [choice, setChoice] = useState(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const client = analytics();
    const saved = client.choice();
    setChoice(saved);
    setOpen(!saved);
    client.start();
  }, []);

  function choose(value) {
    analytics().choose(value);
    setChoice(value);
    setOpen(false);
  }

  return <>
    <button className="analytics-settings caption" type="button" aria-expanded={open}
      aria-controls="analytics-choice" onClick={() => setOpen(!open)}>Analytics preferences</button>
    {open && <section className="analytics-choice" id="analytics-choice" aria-labelledby="analytics-title">
      <h2 id="analytics-title">Help us understand what works.</h2>
      <p>With your permission, Google Analytics measures visits and download clicks using cookies. Downloads work either way. We save your choice on this device; you can change it here anytime.</p>
      <details><summary>What gets measured?</summary><p>Google receives page visits, device and browser information, and the version, platform and file clicked. We do not send torrent searches, magnet links or activity inside the app. This measures clicks, not completed downloads. <a href="https://policies.google.com/privacy">Google’s privacy policy</a>.</p></details>
      <div className="analytics-actions">
        <button className="button secondary-button" type="button" onClick={() => choose('declined')}>{choice === 'accepted' ? 'Turn off analytics' : 'Decline analytics'}</button>
        <button className="button secondary-button" type="button" onClick={() => choose('accepted')}>Allow analytics</button>
        {choice && <button className="analytics-settings" type="button" onClick={() => setOpen(false)}>Keep current choice</button>}
      </div>
    </section>}
  </>;
}
