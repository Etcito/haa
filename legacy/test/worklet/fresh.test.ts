// Q3: the committed worklet bundle equals a fresh build.
import { readFileSync } from 'node:fs';
import { expect, test } from 'vitest';
import { buildWorkletText } from '../../scripts/build-worklet.ts';

test('committed worklet bundle is fresh', async () => {
  // Windows checkouts (core.autocrlf) turn the committed bundle into CRLF; compare line endings as LF.
  expect(await buildWorkletText()).toBe(readFileSync('worklet/haa-processors.js', 'utf8').replace(/\r\n?/g, '\n'));
});
