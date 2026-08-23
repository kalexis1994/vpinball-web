import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './styles.css';

const root = document.getElementById('root');
if (!root) throw new Error('missing #root in index.html');

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// The service worker, which is what makes the page installable and what lets it
// run with no network at all. It is emitted by the `vpw-pwa` plugin in
// `vite.config.ts`; see there for why it caches the way it does.
//
// Development is left alone on purpose. A service worker that serves the last
// build from a cache is the single most confusing thing that can happen while
// somebody is editing the thing it cached.
if (import.meta.env.PROD && 'serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    const base = import.meta.env.BASE_URL;
    navigator.serviceWorker.register(`${base}sw.js`, { scope: base }).catch((e: unknown) => {
      // Not fatal, and not worth a dialog: the page works, it just will not
      // work on a plane. Private windows and some corporate policies refuse
      // registration outright.
      console.warn('the service worker did not register:', e);
    });
  });
}
