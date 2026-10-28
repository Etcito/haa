/** Stateless arithmetic and value shaping for reusable modulation signals. */
export const MATH_PARAMS = {
  a: [-20000, 20000, 0],
  b: [-20000, 20000, 1],
  edge0: [-20000, 20000, 0],
  edge1: [-20000, 20000, 1],
};

export const MATH_ENUMS = {
  operation: ['multiply', 'add', 'subtract', 'divide', 'min', 'max', 'smoothstep'],
};

// Called in the audio sample loop. Existing arithmetic retains its exact order.
export function mathSample(p, operation) {
  if (operation === 'multiply') return p.a * p.b;
  if (operation === 'add') return p.a + p.b;
  if (operation === 'subtract') return p.a - p.b;
  if (operation === 'divide') return Math.abs(p.b) > 1e-8 ? p.a / p.b : 0;
  if (operation === 'min') return Math.min(p.a, p.b);
  if (operation === 'smoothstep') {
    const range = p.edge1 - p.edge0;
    // Coincident edges are an explicit hard step. Reversing distinct edges
    // gives a descending transition without changing their authored values.
    if (range === 0) return p.a < p.edge0 ? 0 : 1;
    const t = Math.min(1, Math.max(0, (p.a - p.edge0) / range));
    return t * t * (3 - 2 * t);
  }
  return Math.max(p.a, p.b);
}
