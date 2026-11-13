import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['test/**/*.test.ts', 'examples/*/test/**/*.test.ts'],
    pool: 'forks',
    maxWorkers: Number(process.env.NMC_CPUS) || 1,
    api: { host: '127.0.0.1' },
  },
  resolve: {
    conditions: ['haa-source'],
  },
});
