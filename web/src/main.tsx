import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './styles.css';

// The one line that answers "which build is this tab actually running?" —
// which, across two machines and a phone, is the hardest question in a
// debugging session. If a fix "did not work", check this first.
console.info(`[build] ${__VPW_BUILD__}`);

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
/**
 * Announces a new build, and applies it when the player says so.
 *
 * A waiting service worker takes over on its own only once every tab running
 * the old build has gone. That is the right default and the wrong behaviour to
 * leave a player with: on a phone nobody closes a tab, they reload, and a
 * reload is precisely what does not help — so without this a player stays on an
 * old build indefinitely while being told the thing is fixed.
 *
 * The reload happens on `controllerchange` rather than straight after the
 * message, because that is the event that says the new worker is actually in
 * charge; reloading sooner just reloads the old one again.
 */
function watchForUpdates(registration: ServiceWorkerRegistration): void {
  let reloading = false;
  navigator.serviceWorker.addEventListener('controllerchange', () => {
    if (reloading) return;
    reloading = true;
    window.location.reload();
  });

  const offer = (worker: ServiceWorker | null) => {
    // Only when one is already in charge: the first install of all is not an
    // update, and asking about it would be asking about nothing.
    if (!worker || !navigator.serviceWorker.controller) return;
    window.dispatchEvent(
      new CustomEvent('vpw-update', { detail: () => worker.postMessage('take-over') }),
    );
  };

  offer(registration.waiting);
  registration.addEventListener('updatefound', () => {
    const installing = registration.installing;
    installing?.addEventListener('statechange', () => {
      if (installing.state === 'installed') offer(installing);
    });
  });
}

// The dev server evicts any service worker squatting on its origin. A machine
// that once opened a *production* build on this same address keeps that
// build's worker registered, and the worker serves its cached shell over
// whatever the dev server has — which reads as "I reloaded and nothing
// changed", for days. One reload after this runs, the origin is clean.
if (import.meta.env.DEV && 'serviceWorker' in navigator) {
  void navigator.serviceWorker.getRegistrations().then((registrations) => {
    for (const r of registrations) {
      console.warn('[dev] unregistering a stale service worker; reload once more');
      void r.unregister();
    }
  });
  if ('caches' in window) {
    void caches.keys().then((keys) => {
      for (const k of keys) void caches.delete(k);
    });
  }
}

if (import.meta.env.PROD && 'serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    const base = import.meta.env.BASE_URL;
    navigator.serviceWorker
      .register(`${base}sw.js`, { scope: base })
      .then((registration) => watchForUpdates(registration))
      .catch((e: unknown) => {
        // Not fatal, and not worth a dialog: the page works, it just will not
        // work on a plane. Private windows and some corporate policies refuse
        // registration outright.
        console.warn('the service worker did not register:', e);
      });
  });
}
