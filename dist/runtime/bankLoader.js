import { alignVariants } from "../bank/pack.js";
import { renderFamilyVariantsAsync } from "../bank/render.js";
import { getFamily, listFamilies } from "../registry/registry.js";
/** Loads each family once and retains frame counts even after the sink transfers its buffers. */
export class BankLoader {
    ctx;
    manifestUrl;
    preset;
    sink;
    pending = new Map();
    lengths = new Map();
    warned = new Set();
    manifestPromise;
    tail = Promise.resolve();
    /** Construct a browser loader; network activity starts with ensure(). */
    constructor(ctx, manifestUrl, preset, sink) {
        this.ctx = ctx;
        this.manifestUrl = manifestUrl;
        this.preset = preset;
        this.sink = sink;
    }
    /** Queue requested families (all registered families by default); failures never reject. */
    async ensure(ids) {
        try {
            const wanted = ids ?? listFamilies().map(def => def.id);
            const promises = wanted.map(id => {
                let promise = this.pending.get(id);
                if (!promise) {
                    promise = this.tail.then(() => this.load(id)).catch(error => {
                        this.lengths.delete(id);
                        this.warn(`load:${id}`, `family load failed ${id}: ${String(error)}`);
                    });
                    this.pending.set(id, promise);
                    this.tail = promise;
                }
                return promise;
            });
            await Promise.all(promises);
        }
        catch (error) {
            this.warn('ensure', `bank load failed: ${String(error)}`);
        }
    }
    /** Number of loaded variants, or zero until loading succeeds. */
    loaded(id) {
        return this.lengths.get(id)?.length ?? 0;
    }
    /** Variant length in frames at the context sample rate, or zero when unknown. */
    variantLength(id, v) {
        return this.lengths.get(id)?.[v] ?? 0;
    }
    warn(cause, message) {
        if (this.warned.has(cause))
            return;
        this.warned.add(cause);
        console.warn(`[haa] ${message}`);
    }
    manifest() {
        this.manifestPromise ??= (async () => {
            try {
                const url = new URL(this.manifestUrl, document.baseURI);
                const response = await fetch(url);
                if (!response.ok)
                    throw new Error(`HTTP ${response.status}`);
                const manifest = await response.json();
                if (!manifest || !Array.isArray(manifest.families))
                    throw new Error('invalid manifest');
                return { manifest, url };
            }
            catch (error) {
                this.warn('manifest', `manifest unavailable: ${String(error)}`);
                return null;
            }
        })();
        return this.manifestPromise;
    }
    async load(id) {
        const def = getFamily(id);
        if (!def) {
            this.warn(`unknown:${id}`, `unknown family: ${id}`);
            return;
        }
        const source = await this.manifest();
        const entry = source?.manifest.families.find(family => family?.id === id);
        let variants;
        if (entry && source) {
            try {
                const response = await fetch(new URL(entry.file, source.url));
                if (!response.ok)
                    throw new Error(`HTTP ${response.status}`);
                const decoded = await this.ctx.decodeAudioData(await response.arrayBuffer());
                variants = alignVariants(decoded.getChannelData(0), entry, this.ctx.sampleRate);
            }
            catch (error) {
                this.warn(`decode:${id}`, `decode failed ${id}: ${String(error)}`);
            }
        }
        else if (source) {
            this.warn(`missing:${id}`, `family missing from manifest: ${id}`);
        }
        variants ??= await renderFamilyVariantsAsync(def, this.ctx.sampleRate, this.preset, 8, () => performance.now(), () => new Promise(resolve => setTimeout(resolve, 0)));
        this.lengths.set(id, variants.map(variant => variant.length));
        this.sink(id, variants, this.ctx.sampleRate);
    }
}
