// Node render CLI: native haa flags and diagnostics over the offline wasm renderer.
import { readFile } from 'node:fs/promises';
import { loadModule, renderPatch, writeWav } from './node.ts';
import type { RenderOptions } from './offline.ts';

const USAGE = 'usage: haa render <patch.json> --out <file.wav>';
const HELP = `${USAGE} [--seconds S] [--seed N] [--rate HZ]
  --input name=value       Set a number or enum name
  --ramp name=a:b          Ramp an input over the render
  --trigger name@seconds  Fire a trigger (--trigger name=at is also accepted)`;

// Rust's float parser accepts decimal floats and case-insensitive inf/nan,
// but not JS's whitespace, empty strings, or hexadecimal literals.
function floatValue(value: string): number | undefined {
  if (/^[+-]?nan$/i.test(value)) return NaN;
  if (/^[+-]?inf(?:inity)?$/i.test(value)) return value[0] === '-' ? -Infinity : Infinity;
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(value)) return undefined;
  return Number(value);
}

function parseFloatFlag(flag: string, value: string): number {
  const number = floatValue(value);
  if (number === undefined || !Number.isFinite(number)) throw new Error(`bad ${flag}: ${value}`);
  return number;
}

function parseUintFlag(flag: string, value: string): number {
  const number = Number(value);
  if (!/^\+?\d+$/.test(value) || number > 0xffff_ffff) throw new Error(`bad ${flag}: ${value}`);
  return number;
}

function splitOnce(value: string, separator: string): [string, string] | undefined {
  const index = value.indexOf(separator);
  return index < 0 ? undefined : [value.slice(0, index), value.slice(index + 1)];
}

function parseArgs(args: string[]): { patch: string; out: string; options: RenderOptions } {
  let patch: string | undefined;
  let out: string | undefined;
  const options: RenderOptions = {
    inputs: Object.create(null) as Record<string, number | string>,
    ramps: Object.create(null) as Record<string, [number, number]>,
    triggers: [],
  };
  for (let i = 0; i < args.length; i++) {
    const arg = args[i]!;
    if (arg.startsWith('--')) {
      const [flag, inline] = splitOnce(arg, '=') ?? [arg, undefined];
      const value = inline ?? args[++i];
      if (value === undefined) throw new Error(`${flag} needs a value`);
      switch (flag) {
        case '--out': out = value; break;
        case '--seconds': options.seconds = parseFloatFlag(flag, value); break;
        case '--seed': options.seed = parseUintFlag(flag, value); break;
        case '--rate': {
          options.sampleRate = parseUintFlag(flag, value);
          if (options.sampleRate === 0) throw new Error('bad --rate: 0');
          break;
        }
        case '--input': {
          const pair = splitOnce(value, '=');
          if (!pair || !pair[0]) throw new Error(`bad --input (want name=value): ${value}`);
          options.inputs![pair[0]] = floatValue(pair[1]) ?? pair[1];
          break;
        }
        case '--ramp': {
          const pair = splitOnce(value, '=');
          const range = pair && splitOnce(pair[1], ':');
          if (!pair || !pair[0] || !range) throw new Error(`bad --ramp (want name=a:b): ${value}`);
          options.ramps![pair[0]] = [parseFloatFlag(flag, range[0]), parseFloatFlag(flag, range[1])];
          break;
        }
        case '--trigger': {
          const pair = splitOnce(value, '@') ?? splitOnce(value, '=');
          if (!pair || !pair[0] || /[@=]/.test(pair[0] + pair[1])) {
            throw new Error(`bad --trigger (want name@seconds): ${value}`);
          }
          options.triggers!.push({ name: pair[0], at: parseFloatFlag(flag, pair[1]) });
          break;
        }
        default: throw new Error(`unknown option: ${flag}`);
      }
    } else if (arg.startsWith('-') && arg !== '-') {
      throw new Error(`unknown option: ${arg}`);
    } else if (patch === undefined) {
      patch = arg;
    } else {
      throw new Error(`unexpected argument: ${arg}`);
    }
  }
  if (patch === undefined) throw new Error(`${USAGE} [--seconds S] [--seed N] [--rate HZ]`);
  if (out === undefined) throw new Error('render needs --out <file.wav>');
  return { patch, out, options };
}

function reason(error: unknown): string {
  return (error instanceof Error ? error.message : String(error)).replace(/^\[haa\] /, '').replace(/[\r\n\u2028\u2029]+/g, ' ');
}

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  if (args[0] === '--help' || (args[0] === 'render' && args.includes('--help'))) {
    console.log(HELP);
    return;
  }
  if (args[0] === undefined) throw new Error(USAGE);
  if (args[0] !== 'render') throw new Error(`unknown command: ${args[0]}`);
  const { patch, out, options } = parseArgs(args.slice(1));
  let bytes: Uint8Array;
  try {
    bytes = await readFile(patch);
  } catch (error) {
    throw new Error(`cannot read ${patch}: ${reason(error)}`);
  }
  const result = renderPatch(await loadModule(), bytes, options);
  try {
    writeWav(out, result);
  } catch (error) {
    throw new Error(`cannot write ${out}: ${reason(error)}`);
  }
  // Defer warnings until writing succeeds so a failure always has one stderr line.
  for (const issue of result.issues) {
    if (issue.severity !== 'info') {
      console.error(`[haa-node] ${reason(`${issue.severity} ${issue.code} ${issue.path}: ${issue.message}`)}`);
    }
  }
  console.log(`[haa-node] wrote ${out} (${result.left.length} frames, ${result.sampleRate} Hz)`);
}

void main().catch((error: unknown) => {
  console.error(`[haa-node] ${reason(error)}`);
  process.exitCode = 1;
});
