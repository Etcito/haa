// Browser bank loading with sequential, cooperative rendering when baked audio is unavailable.
import type { BankManifest, MixPreset } from '../contract.ts'
import { alignVariants } from '../bank/pack.ts'
import { renderFamilyVariantsAsync } from '../bank/render.ts'
import { getFamily, listFamilies } from '../registry/registry.ts'

type ManifestSource = { manifest: BankManifest; url: URL }

/** Loads each family once and retains frame counts even after the sink transfers its buffers. */
export class BankLoader {
  private readonly ctx: BaseAudioContext
  private readonly manifestUrl: string
  private readonly preset: MixPreset
  private readonly sink: (id: string, variants: Float32Array[], rate: number) => void
  private readonly pending = new Map<string, Promise<void>>()
  private readonly lengths = new Map<string, number[]>()
  private readonly warned = new Set<string>()
  private manifestPromise: Promise<ManifestSource | null> | undefined
  private tail: Promise<void> = Promise.resolve()

  /** Construct a browser loader; network activity starts with ensure(). */
  constructor(
    ctx: BaseAudioContext,
    manifestUrl: string,
    preset: MixPreset,
    sink: (id: string, variants: Float32Array[], rate: number) => void,
  ) {
    this.ctx = ctx
    this.manifestUrl = manifestUrl
    this.preset = preset
    this.sink = sink
  }

  /** Queue requested families (all registered families by default); failures never reject. */
  async ensure(ids?: readonly string[]): Promise<void> {
    try {
      const wanted = ids ?? listFamilies().map(def => def.id)
      const promises = wanted.map(id => {
        let promise = this.pending.get(id)
        if (!promise) {
          promise = this.tail.then(() => this.load(id)).catch(error => {
            this.lengths.delete(id)
            this.warn(`load:${id}`, `family load failed ${id}: ${String(error)}`)
          })
          this.pending.set(id, promise)
          this.tail = promise
        }
        return promise
      })
      await Promise.all(promises)
    } catch (error) {
      this.warn('ensure', `bank load failed: ${String(error)}`)
    }
  }

  /** Number of loaded variants, or zero until loading succeeds. */
  loaded(id: string): number {
    return this.lengths.get(id)?.length ?? 0
  }

  /** Variant length in frames at the context sample rate, or zero when unknown. */
  variantLength(id: string, v: number): number {
    return this.lengths.get(id)?.[v] ?? 0
  }

  private warn(cause: string, message: string): void {
    if (this.warned.has(cause)) return
    this.warned.add(cause)
    console.warn(`[haa] ${message}`)
  }

  private manifest(): Promise<ManifestSource | null> {
    this.manifestPromise ??= (async () => {
      try {
        const url = new URL(this.manifestUrl, document.baseURI)
        const response = await fetch(url)
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        const manifest = await response.json() as BankManifest
        if (!manifest || !Array.isArray(manifest.families)) throw new Error('invalid manifest')
        return { manifest, url }
      } catch (error) {
        this.warn('manifest', `manifest unavailable: ${String(error)}`)
        return null
      }
    })()
    return this.manifestPromise
  }

  private async load(id: string): Promise<void> {
    const def = getFamily(id)
    if (!def) {
      this.warn(`unknown:${id}`, `unknown family: ${id}`)
      return
    }
    const source = await this.manifest()
    const entry = source?.manifest.families.find(family => family?.id === id)
    let variants: Float32Array[] | undefined
    if (entry && source) {
      try {
        const response = await fetch(new URL(entry.file, source.url))
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        const decoded = await this.ctx.decodeAudioData(await response.arrayBuffer())
        variants = alignVariants(decoded.getChannelData(0), entry, this.ctx.sampleRate)
      } catch (error) {
        this.warn(`decode:${id}`, `decode failed ${id}: ${String(error)}`)
      }
    } else if (source) {
      this.warn(`missing:${id}`, `family missing from manifest: ${id}`)
    }
    variants ??= await renderFamilyVariantsAsync(
      def, this.ctx.sampleRate, this.preset, 8,
      () => performance.now(), () => new Promise<void>(resolve => setTimeout(resolve, 0)),
    )
    this.lengths.set(id, variants.map(variant => variant.length))
    this.sink(id, variants, this.ctx.sampleRate)
  }
}
