// Browser runtime: wires the HAA AudioWorklet nodes to a Conductor and a BankLoader.
// Browser-only (Web Audio API, timers); the control brain itself stays in conductor.ts.
import { BUS_INDEX, DEFAULT_USER_VOLUMES, MIX_INPUTS, PROCESSOR_NAMES, REVERB_INPUT } from "../contract.js";
import { registerBuiltins } from "../registry/builtins.js";
import { activeMix, getMix } from "../registry/registry.js";
import { BankLoader } from "./bankLoader.js";
import { Conductor } from "./conductor.js";
const ANALYSER_FFT = 2048;
const DISCONNECT_MS = 100;
function unavailable(reason) {
    return new Error(`[haa] worklet unavailable: ${reason instanceof Error ? reason.message : String(reason)}`);
}
function resolveMix(mix) {
    if (mix === undefined)
        return activeMix();
    if (typeof mix !== 'string')
        return mix;
    const preset = getMix(mix);
    if (!preset)
        throw new Error(`[haa] unknown mix ${mix}`);
    return preset;
}
function node(ctx, name, options) {
    try {
        return new AudioWorkletNode(ctx, name, options);
    }
    catch (error) {
        throw unavailable(error);
    }
}
function wait(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}
/**
 * Creates the browser audio engine on a context: mix, sampler and per-bed worklet nodes driven by a
 * Conductor. Rejects '[haa] worklet unavailable: <reason>' only when the worklet cannot be set up.
 */
export async function createAudio(ctx, o) {
    registerBuiltins();
    const preset = resolveMix(o.mix);
    if (!ctx.audioWorklet)
        throw new Error('[haa] worklet unavailable: no audioWorklet');
    try {
        await ctx.audioWorklet.addModule(o.processorUrl);
    }
    catch (error) {
        throw unavailable(error);
    }
    const mix = node(ctx, PROCESSOR_NAMES.mix, {
        numberOfInputs: MIX_INPUTS, numberOfOutputs: 1, outputChannelCount: [2],
        channelCount: 2, channelCountMode: 'explicit', processorOptions: { preset },
    });
    const nodes = [mix];
    let meters = null;
    if (typeof AudioContext !== 'undefined' && ctx instanceof AudioContext) {
        const analyser = () => {
            const a = ctx.createAnalyser();
            a.fftSize = ANALYSER_FFT;
            nodes.push(a);
            return a;
        };
        const splitter = ctx.createChannelSplitter(2);
        nodes.push(splitter);
        meters = { mix: analyser(), left: analyser(), right: analyser() };
        mix.connect(splitter);
        splitter.connect(meters.left, 0);
        splitter.connect(meters.right, 1);
        mix.connect(meters.mix);
    }
    mix.connect(ctx.destination);
    const sampler = node(ctx, PROCESSOR_NAMES.sampler, {
        numberOfInputs: 0, numberOfOutputs: MIX_INPUTS,
        outputChannelCount: Array.from({ length: MIX_INPUTS }, () => 2),
    });
    nodes.push(sampler);
    for (let i = 0; i < MIX_INPUTS; i++)
        sampler.connect(mix, i, i);
    const postMix = (m) => mix.port.postMessage(m);
    const postSampler = (m) => {
        if (m.t === 'load')
            sampler.port.postMessage(m, m.variants.map((v) => v.buffer));
        else
            sampler.port.postMessage(m);
    };
    postMix({ t: 'preset', p: preset });
    postMix({ t: 'user', ...DEFAULT_USER_VOLUMES });
    const beds = new Map();
    let bedWarned = false;
    const dropBed = (handle) => {
        const bed = beds.get(handle);
        if (!bed)
            return;
        beds.delete(handle);
        bed.disconnect();
    };
    const backend = {
        sampler: postSampler,
        mix: postMix,
        startBed(handle, def, seed, trimDb, values) {
            let bed;
            try {
                bed = new AudioWorkletNode(ctx, PROCESSOR_NAMES.bed, {
                    numberOfInputs: 0, numberOfOutputs: 2, outputChannelCount: [2, 2],
                    processorOptions: { bed: def.id, seed, trimDb, values: Array.from(values) },
                });
            }
            catch (error) {
                if (!bedWarned)
                    console.warn(`[haa] bed node failed ${def.id}: ${String(error)}`);
                bedWarned = true;
                return false;
            }
            bed.connect(mix, 0, BUS_INDEX[def.bus]);
            bed.connect(mix, 1, REVERB_INPUT);
            beds.set(handle, bed);
            return true;
        },
        bed(handle, m) {
            const bed = beds.get(handle);
            if (!bed)
                return;
            bed.port.postMessage(m);
            if (m.t === 'end')
                setTimeout(() => dropBed(handle), m.fade * 1000 + DISCONNECT_MS);
        },
    };
    const loader = new BankLoader(ctx, o.manifestUrl, preset, (id, variants, rate) => {
        postSampler({ t: 'load', id, variants, rate });
    });
    const conductor = new Conductor({
        preset,
        maxListeners: o.maxListeners ?? 1,
        seed: o.seed ?? 1,
        spatial: o.spatial ?? null,
        backend,
        now: () => ctx.currentTime,
        loaded: (id) => loader.loaded(id),
        variantLength: (id, v) => loader.variantLength(id, v),
        loadRate: ctx.sampleRate,
    });
    const ready = loader.ensure(o.families).catch((error) => {
        console.warn(`[haa] bank load failed: ${String(error)}`);
    });
    let closing = null;
    const close = async () => {
        postSampler({ t: 'stopAll', fade: 0.05 });
        for (const bed of beds.values())
            bed.port.postMessage({ t: 'end', fade: 0.05 });
        await wait(DISCONNECT_MS);
        for (const bed of beds.values())
            bed.disconnect();
        beds.clear();
        for (const n of nodes)
            n.disconnect();
    };
    return {
        ctx,
        meters,
        ready: () => ready,
        close: () => (closing ??= close().catch(() => { })),
        play: (id, opts) => conductor.play(id, opts),
        startBed: (id, opts) => conductor.startBed(id, opts),
        setListeners: (ls) => conductor.setListeners(ls),
        setSpatial: (m) => conductor.setSpatial(m),
        setOcclusion: (fn) => conductor.setOcclusion(fn),
        duck: (kind, strength) => conductor.duck(kind, strength),
        setVolumes: (v) => conductor.setVolumes(v),
        setReverb: (p) => conductor.setReverb(p),
        setMuffle: (v) => conductor.setMuffle(v),
        get music() { return conductor.music; },
    };
}
