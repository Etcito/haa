// Builds dist/ (the published package): JS ESM and .d.ts for the root entry and ./node.
// tsc emits the JavaScript with rewritten relative import extensions (tsconfig.build.json); the
// declarations keep the source './x.ts' specifiers, so they are rewritten to './x.js' here.
// Usage: node scripts/build-dist.ts
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('../', import.meta.url));
const DIST = join(ROOT, 'dist');
const TSC = join(ROOT, 'node_modules', 'typescript', 'bin', 'tsc');

/** Every .d.ts file under dir. */
function declarations(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return declarations(path);
    return name.endsWith('.d.ts') ? [path] : [];
  });
}

/** Rewrites relative './x.ts' and '../x.ts' specifiers (import, export from, import()) to '.js'. */
export function rewriteDeclarationSpecifiers(text: string): string {
  return text.replace(/(\bfrom\s+|\bimport\s*\(\s*|\bimport\s+)(['"])(\.{1,2}\/[^'"]+?)\.ts\2/g, '$1$2$3.js$2');
}

execFileSync(process.execPath, [TSC, '-p', join(ROOT, 'tsconfig.build.json')], { stdio: 'inherit', cwd: ROOT });
for (const file of declarations(DIST)) {
  const before = readFileSync(file, 'utf8');
  const after = rewriteDeclarationSpecifiers(before);
  if (after !== before) writeFileSync(file, after);
}
console.log(`[build-dist] wrote ${DIST}`);
