import { useEffect, useState } from 'react';

export function useSiteMotion() {
  const [reduced, setReduced] = useState(false);

  function replayAssembly() {
    const assembly = document.querySelector('.assembly');
    assembly.classList.remove('is-playing');
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    void assembly.getBoundingClientRect();
    assembly.classList.add('is-playing');
  }

  useEffect(() => {
    const preference = matchMedia('(prefers-reduced-motion: reduce)');
    function updatePreference() {
      document.getAnimations().forEach(animation => animation.cancel());
      setReduced(preference.matches);
      replayAssembly();
    }
    preference.addEventListener('change', updatePreference);
    updatePreference();
    let reveal;
    let statement;
    if ('IntersectionObserver' in window) {
      reveal = new IntersectionObserver(entries => {
        for (const entry of entries) if (entry.isIntersecting) {
          if (!preference.matches) entry.target.animate([
            { opacity: .4, transform: 'translateY(24px)' }, { opacity: 1, transform: 'translateY(0)' },
          ], { duration: 650, easing: 'cubic-bezier(.22,1,.36,1)' });
          reveal.unobserve(entry.target);
        }
      }, { threshold: .12 });
      document.querySelectorAll('[data-reveal]').forEach(element => reveal.observe(element));
      const words = [...document.querySelectorAll('[data-word]')];
      statement = new IntersectionObserver(entries => {
        if (!entries.some(entry => entry.isIntersecting)) return;
        if (!preference.matches) words.forEach((word, index) => word.animate([
          { opacity: .35 }, { opacity: 1 },
        ], { duration: 650, delay: index * 140, easing: 'cubic-bezier(.22,1,.36,1)' }));
        statement.disconnect();
      }, { threshold: .5 });
      statement.observe(document.querySelector('.control-statement'));
    }
    return () => {
      preference.removeEventListener('change', updatePreference);
      reveal?.disconnect();
      statement?.disconnect();
      document.getAnimations().forEach(animation => animation.cancel());
    };
  }, []);

  return { reduced, replayAssembly };
}
