import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { extname, join, resolve } from 'node:path';
import { defineConfig, type Plugin } from 'vite';
import react from '@vitejs/plugin-react';

/** Folder with the table and the ROM that `/debug` uses. Not in the repo: too big. */
const DEBUG_DIR = resolve(__dirname, 'debug-assets');

const TYPES: Record<string, string> = {
  '.vpx': 'application/octet-stream',
  '.zip': 'application/zip',
  '.json': 'application/json',
};

/**
 * Serves `web/debug-assets/` under `/debug-assets/`, in development only.
 *
 * It exists so that you can go to `/debug` and get a table —with its ROM— on
 * screen without going through the menu or loading it into IndexedDB on every
 * test. The folder is in `.gitignore` because a real table weighs more than
 * 100 MB.
 *
 * Requesting `/debug-assets/` with no name returns the listing as JSON, so the
 * front end discovers what is there without anyone hardcoding file names.
 */
function debugAssets(): Plugin {
  return {
    name: 'vpw-assets-debug',
    apply: 'serve',
    configureServer(server) {
      server.middlewares.use('/debug-assets', (req, res) => {
        const name = decodeURIComponent((req.url ?? '/').split('?')[0]).replace(/^\//, '');

        if (!existsSync(DEBUG_DIR)) {
          res.statusCode = 404;
          res.end(`${DEBUG_DIR} does not exist; put the table and the rom there`);
          return;
        }

        if (name === '') {
          // Files only: `telemetry/` lives in here too, and a directory is not
          // something the front end can load.
          const files = readdirSync(DEBUG_DIR)
            .filter((f) => !f.startsWith('.'))
            .map((f) => ({ name: f, stat: statSync(join(DEBUG_DIR, f)) }))
            .filter((f) => f.stat.isFile())
            .map((f) => ({ name: f.name, size: f.stat.size }));
          res.setHeader('Content-Type', 'application/json');
          res.end(JSON.stringify(files));
          return;
        }

        // No climbing out of the directory.
        const path = resolve(DEBUG_DIR, name);
        if (!path.startsWith(DEBUG_DIR) || !existsSync(path)) {
          res.statusCode = 404;
          res.end(`${name} is not in debug-assets/`);
          return;
        }

        const info = statSync(path);
        res.setHeader(
          'Content-Type',
          TYPES[extname(path).toLowerCase()] ?? 'application/octet-stream',
        );
        res.setHeader('Content-Length', String(info.size));
        res.end(readFileSync(path));
      });
    },
  };
}

/** Where a mark's telemetry lands. Under `debug-assets/`, so it is git-ignored. */
const TELEMETRY_DIR = resolve(DEBUG_DIR, 'telemetry');

/**
 * Takes a telemetry dump and writes it next to the table, in development only.
 *
 * The browser already downloads the same bytes, so this is not what makes the
 * feature work — it is what removes the step where somebody has to find the
 * file and send it. Pressing the mark key puts it in the repo, and whoever is
 * debugging reads it from there.
 */
function telemetrySink(): Plugin {
  return {
    name: 'vpw-telemetry-sink',
    apply: 'serve',
    configureServer(server) {
      server.middlewares.use('/debug-telemetry', (req, res) => {
        if (req.method !== 'POST') {
          res.statusCode = 405;
          res.end('POST a telemetry dump here');
          return;
        }
        // Only the last segment, and only the characters a dump's name has.
        // The name comes from the page, and a page is not somewhere to take a
        // file path from.
        const asked = decodeURIComponent((req.url ?? '/').split('?')[0]).replace(/^\//, '');
        const name = /^[A-Za-z0-9._-]+\.json$/.test(asked)
          ? asked
          : `telemetry-${Date.now()}.json`;

        const chunks: Buffer[] = [];
        req.on('data', (c: Buffer) => chunks.push(c));
        req.on('end', () => {
          try {
            mkdirSync(TELEMETRY_DIR, { recursive: true });
            const path = join(TELEMETRY_DIR, name);
            writeFileSync(path, Buffer.concat(chunks));
            server.config.logger.info(`telemetry -> ${path}`);
            res.statusCode = 200;
            res.end(path);
          } catch (e) {
            res.statusCode = 500;
            res.end(String(e));
          }
        });
      });
    },
  };
}


/**
 * Makes the page installable and able to run with no network.
 *
 * Written here rather than taken from a plugin because the whole of it is one
 * manifest and one service worker, and both have to know things only the build
 * knows: what `base` came out as, and what the emitted files ended up being
 * called. A generic plugin's job is mostly to work that out.
 *
 * Offline matters more here than for most pages. A table and its ROM are
 * already in IndexedDB — the player put them there — so the only thing standing
 * between somebody on a plane and a game of pinball is six megabytes of
 * WebAssembly that they have already downloaded once.
 *
 * # Why the assets can be cached first and asked about never
 *
 * Everything Vite emits carries a hash of its own contents in its name, so a
 * given URL's bytes can never change. That makes cache-first correct rather
 * than merely fast, and it makes the update story safe by construction: the
 * JavaScript glue wasm-bindgen writes only ever asks for the exact `.wasm` it
 * was generated against, and a stale pairing of the two cannot be assembled
 * out of a cache keyed by name.
 *
 * The service worker deliberately does **not** call `skipWaiting`. A new build
 * waits until every tab running the old one has gone, so a running game is
 * never served half of one version and half of another.
 */
function pwa(): Plugin {
  const base = process.env.VPW_BASE ?? '/';
  let assets: string[] = [];

  return {
    name: 'vpw-pwa',
    apply: 'build',

    generateBundle(_options, bundle) {
      // The shell: everything the page needs before it can show anything. The
      // table images and the ROMs are not here and never will be — they are the
      // player's own files and they live in IndexedDB.
      assets = Object.keys(bundle).filter((name) => !name.endsWith('.map'));

      this.emitFile({
        type: 'asset',
        fileName: 'manifest.webmanifest',
        source: JSON.stringify(
          {
            // Pinned so the identity survives the site moving path.
            id: 'vpinball-web',
            name: 'Visual Pinball',
            short_name: 'Pinball',
            description:
              'Visual Pinball tables, played in the browser. Bring your own .vpx and ROM.',
            start_url: base,
            scope: base,
            // Fullscreen where it is offered, because the point of the overhead
            // view is that the screen is the glass over the playfield and a
            // browser chrome across the top of it is a browser chrome across
            // the top of the playfield. Standalone everywhere else.
            display: 'standalone',
            display_override: ['fullscreen'],
            // A table is twice as long as it is wide, and so is a phone held
            // upright. Turning it sideways does not show more table, it shows
            // less.
            orientation: 'portrait',
            background_color: '#0b1020',
            theme_color: '#0b1020',
            categories: ['games'],
            icons: [
              { src: `${base}icon-192.png`, sizes: '192x192', type: 'image/png' },
              { src: `${base}icon-512.png`, sizes: '512x512', type: 'image/png' },
              // Android crops this one to whatever shape it likes and only
              // promises the middle 80% survives, so it is drawn to suit.
              {
                src: `${base}icon-maskable-512.png`,
                sizes: '512x512',
                type: 'image/png',
                purpose: 'maskable',
              },
            ],
          },
          null,
          2,
        ),
      });
    },

    // `writeBundle` rather than `generateBundle`: the icons live in `public/`
    // and Vite copies those in afterwards, so this is the first point at which
    // the full list of what will be on the server is known.
    writeBundle() {
      const shell = [
        base,
        ...assets.map((a) => base + a),
        `${base}manifest.webmanifest`,
        `${base}icon-192.png`,
        `${base}icon-512.png`,
        `${base}icon-maskable-512.png`,
        `${base}apple-touch-icon.png`,
      ];
      // The cache is named after what is in it, so a build that changed nothing
      // reuses the cache and a build that changed anything gets a new one and
      // sweeps the old away.
      const version = createHash('sha256').update(shell.join('\n')).digest('hex').slice(0, 12);
      writeFileSync(resolve(__dirname, 'dist/sw.js'), serviceWorker(version, base, shell));
    },
  };
}

/** The service worker itself. See {@link pwa} for why it is shaped like this. */
function serviceWorker(version: string, base: string, shell: string[]): string {
  return `// Generated by the vpw-pwa plugin in vite.config.ts. Do not edit.
const CACHE = 'vpw-${version}';
const SHELL = ${JSON.stringify(shell, null, 2)};

self.addEventListener('install', (event) => {
  // Not skipWaiting: a new build takes over only once every tab running the old
  // one is gone, so a game in progress is never handed half of each.
  event.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)));
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((names) =>
        Promise.all(names.filter((n) => n.startsWith('vpw-') && n !== CACHE).map((n) => caches.delete(n))),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET') return;

  const url = new URL(request.url);
  if (url.origin !== self.location.origin || !url.pathname.startsWith(${JSON.stringify(base)})) {
    return;
  }

  // A navigation is the one request whose URL says nothing about its contents:
  // every route in the app is the same document. Try the network so a new build
  // is picked up, and fall back to the copy that is already here.
  if (request.mode === 'navigate') {
    event.respondWith(
      fetch(request).catch(() => caches.match(${JSON.stringify(base)}).then((r) => r ?? Response.error())),
    );
    return;
  }

  // Everything else carries a content hash in its name, so what is in the cache
  // under a given URL is what that URL means, for ever.
  event.respondWith(
    caches.match(request).then(
      (hit) =>
        hit ??
        fetch(request).then((response) => {
          if (response.ok && response.type === 'basic') {
            const copy = response.clone();
            caches.open(CACHE).then((c) => c.put(request, copy));
          }
          return response;
        }),
    ),
  );
});
`;
}

export default defineConfig({
  // GitHub Pages serves a project site from `/<repo>/`, not from the root, so
  // every asset URL has to carry that prefix. Taken from the environment rather
  // than written in: the dev server and a local `vite preview` both live at the
  // root, and hardcoding the repository name would break both.
  base: process.env.VPW_BASE ?? '/',
  plugins: [react(), debugAssets(), telemetrySink(), pwa()],
  server: { port: 8091 },
  // The .wasm comfortably exceeds Vite's inline limit; let it be served as a
  // separate file so the browser can cache it.
  build: { assetsInlineLimit: 0 },
});
