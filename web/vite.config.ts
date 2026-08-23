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

export default defineConfig({
  // GitHub Pages serves a project site from `/<repo>/`, not from the root, so
  // every asset URL has to carry that prefix. Taken from the environment rather
  // than written in: the dev server and a local `vite preview` both live at the
  // root, and hardcoding the repository name would break both.
  base: process.env.VPW_BASE ?? '/',
  plugins: [react(), debugAssets(), telemetrySink()],
  server: { port: 8091 },
  // The .wasm comfortably exceeds Vite's inline limit; let it be served as a
  // separate file so the browser can cache it.
  build: { assetsInlineLimit: 0 },
});
