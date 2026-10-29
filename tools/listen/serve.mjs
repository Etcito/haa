// Loopback dev server for the listen page (ARCHITECTURE listen module): checks the built
// worklet and wasm, bundles tools/listen/main.ts into out/listen/main.js with esbuild, then
// serves the repository root on 127.0.0.1 so the page can load patches, the package and the
// Sona reference by URL.

import { build } from 'esbuild';
import { createReadStream } from 'node:fs';
import { mkdir, readdir, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

/** Repository root: tools/listen/serve.mjs -> ../.. */
const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const WORKLET = path.join(ROOT, 'packages/haa/dist/worklet.js');
const WASM = path.join(ROOT, 'packages/haa/pkg/haa.wasm');
const PAGE_SOURCE = path.join(ROOT, 'tools/listen/main.ts');
const PAGE_BUNDLE = path.join(ROOT, 'out/listen/main.js');
/** Files the page loads through static routes, by extension. */
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.wasm': 'application/wasm',
  '.wav': 'audio/wav',
};
const DEFAULT_TYPE = 'application/octet-stream';
const PATCH_ROOT = path.join(ROOT, 'patches');

/** True when the file exists and is not a directory. */
async function isFile(file) {
  try {
    return (await stat(file)).isFile();
  } catch {
    return false;
  }
}

/** Repo-relative JSON paths under patches/, recursively, sorted with forward slashes. */
async function listPatches() {
  const found = [];
  async function walk(directory) {
    let entries;
    try {
      entries = await readdir(directory, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        await walk(full);
      } else if (entry.isFile() && entry.name.endsWith('.json')) {
        found.push(path.relative(ROOT, full).split(path.sep).join('/'));
      }
    }
  }
  await walk(PATCH_ROOT);
  found.sort();
  return found;
}

/** Send a short text response with the common no-store header. */
function finish(res, status, body = '', extra = {}) {
  res.writeHead(status, {
    'Cache-Control': 'no-store',
    'Content-Type': 'text/plain; charset=utf-8',
    ...extra,
  });
  res.end(body);
}

/** Map a decoded URL pathname to a file inside the root, or null when it escapes. */
function resolveRequest(root, pathname) {
  let decoded;
  try {
    decoded = decodeURIComponent(pathname);
  } catch {
    return null;
  }
  if (decoded.includes('\0')) return null;
  const target = path.resolve(root, decoded.replace(/^\/+/, ''));
  const relative = path.relative(root, target);
  if (relative.startsWith('..') || path.isAbsolute(relative)) return null;
  return target;
}

/** Static file response, or 404 when the path is missing or a directory. */
async function serveStatic(root, pathname, method, res) {
  const target = resolveRequest(root, pathname);
  if (target === null) {
    finish(res, 403, 'Forbidden');
    return;
  }
  let info;
  try {
    info = await stat(target);
  } catch {
    finish(res, 404, 'Not found');
    return;
  }
  if (info.isDirectory()) {
    finish(res, 404, 'Not found');
    return;
  }
  const type = MIME[path.extname(target).toLowerCase()] ?? DEFAULT_TYPE;
  res.writeHead(200, {
    'Cache-Control': 'no-store',
    'Content-Type': type,
    'Content-Length': info.size,
  });
  if (method === 'HEAD') {
    res.end();
    return;
  }
  const stream = createReadStream(target);
  stream.on('error', () => res.destroy());
  stream.pipe(res);
}

/** Handle one request to the listen server. */
async function handle(root, req, res) {
  const method = req.method ?? 'GET';
  if (method !== 'GET' && method !== 'HEAD') {
    finish(res, 405, 'Method not allowed', { Allow: 'GET, HEAD' });
    return;
  }
  let pathname;
  try {
    pathname = new URL(req.url ?? '/', 'http://127.0.0.1').pathname;
  } catch {
    finish(res, 400, 'Bad request');
    return;
  }
  if (pathname === '/') {
    finish(res, 302, '', { Location: '/tools/listen/index.html' });
    return;
  }
  if (pathname === '/__patches') {
    const body = JSON.stringify(await listPatches());
    res.writeHead(200, {
      'Cache-Control': 'no-store',
      'Content-Type': 'application/json; charset=utf-8',
    });
    res.end(method === 'HEAD' ? undefined : body);
    return;
  }
  await serveStatic(root, pathname, method, res);
}

/** Bundle tools/listen/main.ts into out/listen/main.js. */
async function bundlePage() {
  await mkdir(path.dirname(PAGE_BUNDLE), { recursive: true });
  await build({
    absWorkingDir: ROOT,
    entryPoints: [PAGE_SOURCE],
    outfile: PAGE_BUNDLE,
    bundle: true,
    format: 'esm',
    target: 'es2022',
    platform: 'browser',
    logLevel: 'silent',
  });
}

/**
 * Build the page, then serve the repository root on 127.0.0.1:port.
 * Resolves with the page URL and an async close function.
 */
export async function startListenServer(port) {
  if (!(await isFile(WORKLET)) || !(await isFile(WASM))) {
    throw new Error('run npm run build first');
  }
  await bundlePage();
  const server = createServer((req, res) => {
    handle(ROOT, req, res).catch(() => {
      if (!res.headersSent) finish(res, 500, 'Internal error');
      else res.destroy();
    });
  });
  await new Promise((resolve, reject) => {
    const onError = (error) => {
      server.off('listening', onListening);
      reject(error);
    };
    const onListening = () => {
      server.off('error', onError);
      resolve();
    };
    server.once('error', onError);
    server.once('listening', onListening);
    server.listen(port, '127.0.0.1');
  });
  const address = server.address();
  const actualPort = typeof address === 'object' && address !== null ? address.port : port;
  const url = `http://127.0.0.1:${actualPort}/`;
  return {
    url,
    close() {
      return new Promise((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()));
        server.closeIdleConnections();
      });
    },
  };
}

const isMain = process.argv[1] !== undefined
  && import.meta.url === pathToFileURL(process.argv[1]).href;

if (isMain) {
  const port = Number(process.env.HAA_LISTEN_PORT) || 8010;
  startListenServer(port).then(
    (server) => {
      console.log(server.url);
    },
    (error) => {
      const reason = error instanceof Error ? error.message : String(error);
      console.error(`[listen] ${reason.replace(/\s*\r?\n\s*/g, ' ')}`);
      process.exit(1);
    },
  );
}
