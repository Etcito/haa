// The few WebGPU names the GPU prototypes use, without the @webgpu/types dependency (the package's
// TS lib has no WebGPU). Constants are the spec's flag values. A production version would take
// @webgpu/types and the real GPUDevice type instead of this loose one.
/* eslint-disable @typescript-eslint/no-explicit-any */
export const BUF = { MAP_READ: 1, COPY_SRC: 4, COPY_DST: 8, UNIFORM: 64, STORAGE: 128, QUERY_RESOLVE: 512 } as const;
export const STAGE_COMPUTE = 4;
export const MAP_READ = 1;

/** A GPUDevice (`navigator.gpu.requestAdapter()` then `requestDevice()`; RAA's own device in Rust is not reachable from JS). */
export type GpuDevice = {
  features: { has(name: string): boolean };
  queue: { writeBuffer(buffer: any, offset: number, data: BufferSource | ArrayBufferView): void; submit(commands: any[]): void; onSubmittedWorkDone(): Promise<void> };
  createShaderModule(d: { code: string }): any;
  createBindGroupLayout(d: any): any;
  createPipelineLayout(d: any): any;
  createComputePipeline(d: any): any;
  createBindGroup(d: any): any;
  createBuffer(d: { size: number; usage: number }): GpuBuffer;
  createQuerySet(d: any): any;
  createCommandEncoder(): any;
};

export type GpuBuffer = {
  mapAsync(mode: number): Promise<void>;
  getMappedRange(): ArrayBuffer;
  unmap(): void;
  destroy(): void;
};
