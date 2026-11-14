// `@etcito/haa/sfx`: physics-driven sound effects (docs/physics-sound.md).
export * from './contract.ts';
export * from './acoustics.ts';
export * from './raytrace.ts';
export * from './faces.ts';
export * from './field.ts';
export * from './field-wasm.ts';
export * from './rapier.ts';
export * from './pool-protocol.ts';
export * from './pool.ts';
export * from './spatial3d.ts';
export * from './swarm.ts';
export * from './engine.ts';
export * from './orchestrator.ts';
export * from './worker-pool.ts';
export * from './mix-pool.ts';
export * from './mix-protocol.ts';
export * from './tracer.ts';
// The tracer's geometry helpers carry surface ids (field-wasm.ts has its own without them).
export { createWasmTracer, geometryFromRapier as tracerGeometryFromRapier } from './trace-wasm.ts';
export type { SfxGeometry as TracerGeometry, WasmTracer } from './trace-wasm.ts';
export * from './trace-worker-client.ts';
export * from './gpu-trace.ts';
export * from './gpu-particles.ts';
