// MP3 bake CLI (bake module, node): registers the built-ins, selects families by id prefix
// and bakes them with bake() into <out>/<id>.mp3 plus manifest.json.
//
//   node scripts/bake.ts [--out <dir>] [--families <comma-separated id prefixes>]
import { bake } from '../src/bake/bake.ts';
import { registerBuiltins } from '../src/registry/builtins.ts';
import { listFamilies } from '../src/registry/registry.ts';

function parseArgs(argv: readonly string[]): { out: string; prefixes: string[] | null } {
  let out = 'out/bake';
  let prefixes: string[] | null = null;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = argv[i + 1];
    if ((arg === '--out' || arg === '--families') && (value === undefined || value.startsWith('--'))) {
      throw new Error(`missing value for ${arg}`);
    }
    if (arg === '--out') out = value;
    else if (arg === '--families') prefixes = value.split(',').map((p) => p.trim()).filter((p) => p.length > 0);
    else throw new Error(`unknown argument ${arg}`);
    i++;
  }
  return { out, prefixes };
}

function main(): void {
  const t0 = performance.now();
  const { out, prefixes } = parseArgs(process.argv.slice(2));
  registerBuiltins();
  const families = listFamilies().filter((f) => prefixes === null || prefixes.some((p) => f.id.startsWith(p)));
  if (families.length === 0) throw new Error(`no family matches ${prefixes === null ? '(all)' : prefixes.join(',')}`);
  bake(families, { outDir: out, log: (line) => console.log(line) });
  console.log(`elapsed ${((performance.now() - t0) / 1000).toFixed(2)} s`);
}

try {
  main();
} catch (err) {
  console.error(`[bake] ${err instanceof Error ? err.message : String(err)}`);
  process.exit(1);
}
