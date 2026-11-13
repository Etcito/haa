import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, extname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const binaryExtensions = new Set(['.wav', '.mp3', '.ogg', '.flac', '.png', '.jpg', '.jpeg', '.webp', '.gif']);
const dspDirectories = ['dsp', 'gen', 'music', 'mix', 'content', 'bank', 'analysis', 'host', 'spatial', 'registry'];
const explicitDspFiles = [
  'src/contract.ts',
  'src/offline/wav.ts', 'src/offline/render.ts', 'src/offline/scene.ts',
  'src/runtime/conductor.ts', 'src/runtime/voices.ts', 'src/runtime/flatSpatial.ts',
];

function walk(directory: string): string[] {
  if (!statExists(directory)) return [];
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? walk(path) : [path];
  });
}

function statExists(path: string): boolean {
  try { return statSync(path).isDirectory(); } catch { return false; }
}

function repoFiles(...directories: string[]): string[] {
  return directories.flatMap((directory) => walk(join(root, directory)));
}

function trackedFiles(): string[] {
  return execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' })
    .split('\0').filter(Boolean);
}

function isDspSafe(path: string): boolean {
  const rel = relative(root, path).split(/[\\/]/).join('/');
  return explicitDspFiles.includes(rel) || dspDirectories.some((directory) =>
    rel.startsWith(`src/${directory}/`) && rel.endsWith('.ts'));
}

function importTokens(source: string): Array<{ specifier: string; statement: string }> {
  const tokens: Array<{ specifier: string; statement: string }> = [];
  const pattern = /\b(import|export)\s+(type\s+)?([\s\S]*?)\s+from\s+(['"])([^'"]+)\4/g;
  for (const match of source.matchAll(pattern)) {
    if (match[2]) continue;
    const clause = match[3].trim();
    const named = clause.match(/^\{([\s\S]*)\}$/);
    if (named && !named[1].split(',').some((item) => item.trim() && !item.trim().startsWith('type '))) continue;
    tokens.push({ specifier: match[5], statement: match[0] });
  }
  const sideEffect = /\bimport\s+(['"])([^'"]+)\1/g;
  for (const match of source.matchAll(sideEffect)) {
    tokens.push({ specifier: match[2], statement: match[0] });
  }
  return tokens;
}

function resolvesToDspSafe(from: string, specifier: string): boolean {
  if (!specifier.startsWith('.')) return false;
  const target = resolve(dirname(from), specifier);
  const candidates = [target, `${target}.ts`, join(target, 'index.ts')];
  return candidates.some((candidate) => isFile(candidate) && isDspSafe(candidate));
}

function isFile(path: string): boolean {
  try { return statSync(path).isFile(); } catch { return false; }
}

describe('repository hygiene', () => {
  it('tracks no audio or image assets', () => {
    expect(trackedFiles().filter((path) => binaryExtensions.has(extname(path).toLowerCase()))).toEqual([]);
  });

  it('contains no three.js or bundler glob imports in source tools and demo', () => {
    const violations = repoFiles('src', 'demo', 'scripts').filter((path) => {
      const text = readFileSync(path, 'utf8');
      return /from\s+['"]three['"]|['"]three\/|import\.meta\.glob/.test(text);
    }).map((path) => relative(root, path));
    expect(violations).toEqual([]);
  });

  it('keeps DSP-safe code isolated and free of host APIs', () => {
    const files = [...new Set([...explicitDspFiles.map((path) => join(root, path)), ...repoFiles('src')])]
      .filter((path) => isFile(path) && path.endsWith('.ts') && isDspSafe(path));
    const forbidden = /Math\.random|Date\.|performance\.|setTimeout|setInterval|document\.|window\.|\b(?:Offline|webkit)?AudioContext\b|new\s+\w*AudioContext\b|from\s+['"]node:/;
    const violations: string[] = [];
    for (const file of files) {
      const source = readFileSync(file, 'utf8');
      const unsafe = forbidden.exec(source);
      if (unsafe) violations.push(`${relative(root, file)}: ${unsafe[0]}`);
      for (const { specifier, statement } of importTokens(source)) {
        if (!resolvesToDspSafe(file, specifier)) {
          const token = statement.match(/['"]([^'"]+)['"]/)?.[1] ?? statement;
          violations.push(`${relative(root, file)}: ${token}`);
        }
      }
    }
    expect(violations).toEqual([]);
  });

  it('imports no absolute paths and no example pack from the core', () => {
    // The core (src, demo, scripts) depends on nothing outside the repository and never imports
    // examples/, which holds optional packs that register themselves through the public API.
    const violations = repoFiles('src', 'demo', 'scripts').flatMap((path) =>
      importTokens(readFileSync(path, 'utf8'))
        .filter(({ specifier }) => isAbsolute(specifier) || /^[A-Za-z]:[\/]/.test(specifier) || /(^|\/)examples\//.test(specifier))
        .map(({ specifier }) => `${relative(root, path)}: ${specifier}`));
    expect(violations).toEqual([]);
  });
});
