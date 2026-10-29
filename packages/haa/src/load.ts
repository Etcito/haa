// Browser main-thread loader for the compiled HAA wasm module (ARCHITECTURE §2, §16).
// Streaming compilation keeps the download and compile overlapped; servers that do not send
// `application/wasm` (or realms without compileStreaming) fall back to an ArrayBuffer compile.

/** Fetch and compile `pkg/haa.wasm`; rejects when the module cannot be fetched or compiled. */
export async function loadHaaModule(url: string | URL): Promise<WebAssembly.Module> {
  if (typeof WebAssembly.compileStreaming === 'function') {
    try {
      return await WebAssembly.compileStreaming(fetch(url));
    } catch (error) {
      if (!(error instanceof TypeError)) throw error;
    }
  }
  const response = await fetch(url);
  if (!response.ok) throw new Error(`[haa] wasm fetch failed: ${response.status} ${String(url)}`);
  return WebAssembly.compile(await response.arrayBuffer());
}
