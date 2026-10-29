// Loopback static server (demo module, node): serves a built directory (default out/demo) on
// 127.0.0.1 only, with a fixed MIME table, 403 for paths escaping the directory, 404 for
// missing files and 405 for methods other than GET/HEAD.
//
//   node scripts/serve.ts [--dir out/demo] [--port 8787]
import { createServer } from 'node:http';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, resolve, sep } from 'node:path';

/** Content type per file extension; every other extension is application/octet-stream. */
const MIME: Record<string, string> = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.json': 'application/json',
  '.mp3': 'audio/mpeg',
  '.wav': 'audio/wav',
  '.png': 'image/png',
};

function send(res: ServerResponse, status: number, text: string): void {
  res.writeHead(status, { 'content-type': 'text/plain' });
  res.end(text);
}

/** Maps a raw request URL to a file path inside root, or returns an HTTP error status. */
function resolvePath(root: string, rawUrl: string): string | number {
  const rawPath = rawUrl.split(/[?#]/, 1)[0];
  let pathname: string;
  try {
    pathname = decodeURIComponent(rawPath);
  } catch {
    return 400;
  }
  // `new URL` would normalize '/../', so check the raw and the decoded segments ourselves.
  if (rawPath.split('/').includes('..') || pathname.split(/[\\/]/).includes('..')) return 403;
  const file = resolve(root, '.' + (pathname.startsWith('/') ? pathname : '/' + pathname));
  if (file !== root && !file.startsWith(root + sep)) return 403;
  return file;
}

async function handle(root: string, req: IncomingMessage, res: ServerResponse): Promise<void> {
  if (req.method !== 'GET' && req.method !== 'HEAD') return send(res, 405, 'Method Not Allowed');
  const target = resolvePath(root, req.url ?? '/');
  if (typeof target === 'number') return send(res, target, target === 403 ? 'Forbidden' : 'Bad Request');
  let file = target;
  try {
    if ((await stat(file)).isDirectory()) file = join(file, 'index.html');
    const body = await readFile(file);
    res.writeHead(200, { 'content-type': MIME[extname(file).toLowerCase()] ?? 'application/octet-stream', 'content-length': body.length });
    res.end(req.method === 'HEAD' ? undefined : body);
  } catch {
    send(res, 404, 'Not Found');
  }
}

/** Serves dir on 127.0.0.1:port (0 = ephemeral); resolves with the bound port and a close(). */
export function startServer(dir: string, port: number): Promise<{ port: number; close(): Promise<void> }> {
  const root = resolve(dir);
  const server = createServer((req, res) => {
    handle(root, req, res).catch(() => send(res, 500, 'Internal Server Error'));
  });
  return new Promise((ok, fail) => {
    server.once('error', fail);
    server.listen(port, '127.0.0.1', () => {
      const address = server.address();
      const bound = typeof address === 'object' && address !== null ? address.port : port;
      ok({
        port: bound,
        close: () => new Promise<void>((done, err) => {
          server.close((e) => (e ? err(e) : done()));
          server.closeAllConnections();
        }),
      });
    });
  });
}

function parseArgs(argv: readonly string[]): { dir: string; port: number } {
  let dir = 'out/demo';
  let port = 8787;
  for (let i = 0; i < argv.length; i += 2) {
    const value = argv[i + 1];
    if (value === undefined) throw new Error(`missing value for ${argv[i]}`);
    if (argv[i] === '--dir') dir = value;
    else if (argv[i] === '--port' && /^\d+$/.test(value)) port = Number(value);
    else throw new Error(`bad argument ${argv[i]} ${value}`);
  }
  return { dir, port };
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === import.meta.filename) {
  try {
    const { dir, port } = parseArgs(process.argv.slice(2));
    const server = await startServer(dir, port);
    console.log(`http://127.0.0.1:${server.port}/`);
  } catch (err) {
    console.error(`[serve] ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  }
}
