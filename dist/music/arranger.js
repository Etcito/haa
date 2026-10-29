// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: registered styles instead of the biome STYLES table, generic section and state names
// The procedural arranger (module music, owner M3). DSP-safe: it imports only ../dsp/rng.ts,
// ../dsp/math.ts and contract types; no unseeded random source, clock, DOM or Web Audio. nextStep
// allocates nothing: the NoteEvents it returns are slots of a pool built in the constructor, so
// out[i] aliases pool[i]. `step` is the absolute 16th-note index of an event and `len` its length
// in 16th notes (a bar is 16). Every pitch is in the style's scale, transposed +2 semitones in the
// climax: the whole scale moves, so the pitches stay in the transposed key.
import { AudioRng, hashSeed } from "../dsp/rng.js";
import { clamp, lerp } from "../dsp/math.js";
const BAR = 16;
const RAMP_STEPS = 32;
const POOL = 32;
const KICK = 36, SNARE = 38, RIM = 37, HAT_C = 42, HAT_O = 46, CRASH = 49;
const TOM_HI = 50, TOM_MID = 47, TOM_LO = 45, SHAKER = 70;
const FILL_TOMS = [TOM_HI, TOM_MID, TOM_LO, TOM_MID];
const SECTIONS = ['intro', 'mainA', 'mainB', 'intense', 'climax', 'resolveWin', 'resolve', 'post'];
/** 4-step rhythm cells as 16th onset masks (bit k = step k of the cell); 0 is a rest cell. */
const CELLS = [0b0001, 0b0101, 0b0111, 0b1001, 0b0100, 0b1101, 0b0011, 0b0000, 0b0001, 0b0101];
const MOTIF_MAX = 32;
const V_ORIG = 0, V_TRANSPOSE = 1, V_DISPLACE = 2, V_INVERT = 3;
export class Arranger {
    rng;
    progRng;
    styles;
    style;
    activeStyle = 0;
    pendingStyle = -1;
    activeIsB = false;
    transpose = 0;
    step = 0;
    bar = 0;
    mainBars = 0;
    introBars = 0;
    finishBar = -1;
    fillBar = false;
    crashBar = false;
    progIndex = 0;
    progChord = 0;
    holdBars = 0;
    chordDegree = 0;
    sectionState = 'intro';
    bpmBase;
    rampFrom;
    bpmValue;
    rampRemaining = 0;
    state = { style: 0, styleB: 0, styleMix: 0, intensity: 0, phase: 0, climax: 0, resolved: 0, won: 0, sparkle: 0, tension: 0 };
    pool = [];
    gateState = { pad: true, bass: false, drums: false, pluck: false, lead: false, counter: false };
    triad = [0, 0, 0];
    padVoices = [0, 0, 0];
    prevPad = [0, 0, 0];
    hasPrevPad = false;
    // Motif (part 2): per-step onset, degree relative to the chord root, length and coverage, all
    // preallocated; seeds per (section, style) are precomputed so nextStep never hashes.
    motifSeeds;
    motifRng = new AudioRng(0);
    mOn = new Uint8Array(MOTIF_MAX);
    mDeg = new Int8Array(MOTIF_MAX);
    mLen = new Uint8Array(MOTIF_MAX);
    mCover = new Uint8Array(MOTIF_MAX);
    mSteps = BAR;
    motifSec = -1;
    motifStyle = -1;
    motifBar = 0;
    varKind = V_ORIG;
    varAmt = 0;
    arpIndex = 0;
    constructor(seed, styles) {
        if (styles.length === 0)
            throw new Error('[haa] no music style');
        this.styles = Object.freeze(styles.slice());
        this.motifSeeds = new Uint32Array(SECTIONS.length * styles.length);
        this.rng = new AudioRng(hashSeed(seed, 'arranger'));
        this.progRng = new AudioRng(hashSeed(seed, 'prog'));
        this.style = styles[0];
        this.bpmBase = this.style.bpm;
        this.rampFrom = this.style.bpm;
        this.bpmValue = this.style.bpm;
        for (let i = 0; i < POOL; i++)
            this.pool.push({ inst: 'pad', midi: 0, vel: 0, step: 0, len: 0 });
        this.progIndex = this.pickProgression();
        for (let i = 0; i < SECTIONS.length; i++) {
            for (let j = 0; j < styles.length; j++)
                this.motifSeeds[i * styles.length + j] = hashSeed(seed, 'motif', i, j);
        }
    }
    get section() { return this.sectionState; }
    get bpm() { return this.bpmValue; }
    get styleIndex() { return this.activeStyle; }
    setState(s) {
        const st = this.state;
        st.style = s.style;
        st.styleB = s.styleB;
        st.styleMix = s.styleMix;
        st.intensity = s.intensity;
        st.phase = s.phase;
        st.climax = s.climax;
        st.resolved = s.resolved;
        st.won = s.won;
        st.sparkle = s.sparkle;
        st.tension = s.tension;
        this.transpose = s.climax !== 0 ? 2 : 0;
        // Dominant style with a 0.1 hysteresis on styleMix; the change lands on the next bar line.
        this.activeIsB = this.activeIsB ? s.styleMix >= 0.4 : s.styleMix > 0.6;
        const idx = clamp((this.activeIsB ? s.styleB : s.style) | 0, 0, this.styles.length - 1);
        this.pendingStyle = idx === this.activeStyle ? -1 : idx;
    }
    nextStep(out) {
        if (this.step === 0)
            this.onBar();
        if (this.rampRemaining > 0) {
            this.bpmBase = lerp(this.rampFrom, this.style.bpm, 1 - this.rampRemaining / RAMP_STEPS);
            this.rampRemaining--;
        }
        else
            this.bpmBase = this.style.bpm;
        this.bpmValue = this.bpmBase * (this.state.climax !== 0 ? 1.06 : 1);
        const g = this.gate(this.state.intensity);
        let n = this.emitSection(out, 0, g);
        n = this.emitMelodicLayers(out, n, g); // lead, pluck/arp and counter-melody
        this.step++;
        if (this.step >= BAR) {
            this.step = 0;
            this.bar++;
        }
        return n;
    }
    /** Bar-quantized state machine, style change, chord and fill decision. */
    onBar() {
        if (this.pendingStyle >= 0 && this.pendingStyle !== this.activeStyle) {
            this.rampFrom = this.bpmBase;
            this.activeStyle = this.pendingStyle;
            this.style = this.styles[this.activeStyle];
            this.pendingStyle = -1;
            this.rampRemaining = RAMP_STEPS;
            this.progIndex = this.pickProgression();
            this.progChord = 0;
            this.holdBars = 0;
        }
        if (this.isFinished() || this.state.climax !== 0) { /* no main bars */ }
        else if (this.state.phase > 0)
            this.mainBars++;
        else
            this.introBars++;
        const sec = this.sectionFor(0);
        const next = this.sectionFor(1);
        this.fillBar = sec !== next && sec !== 'intro' && next !== 'intro' && sec !== 'post' && next !== 'post';
        if (sec !== this.sectionState)
            this.onSectionChange(sec);
        this.sectionState = sec;
        this.advanceChord(sec);
    }
    isFinished() { return this.state.resolved !== 0 || this.state.phase >= 2; }
    sectionFor(offset) {
        const s = this.state;
        if (this.finishBar >= 0 && this.bar + offset - this.finishBar >= 2)
            return 'post';
        if (this.isFinished())
            return s.won !== 0 ? 'resolveWin' : 'resolve';
        if (s.climax !== 0)
            return 'climax';
        if (s.phase <= 0)
            return 'intro';
        if (s.intensity >= 0.85)
            return 'intense';
        return Math.floor((this.mainBars - 1 + offset) / 8) % 2 === 0 ? 'mainA' : 'mainB';
    }
    onSectionChange(sec) {
        if (sec === 'resolveWin' || sec === 'resolve')
            this.finishBar = this.bar;
        if (sec === 'intro')
            this.introBars = 0;
        this.crashBar = sec !== 'intro' && sec !== 'post' && sec !== 'resolve';
        this.progIndex = this.pickProgression();
        this.progChord = 0;
        this.holdBars = 0;
        this.hasPrevPad = false;
    }
    /** Markov choice among the style's progressions: the next one is favoured and an immediate
     *  repeat is forbidden (unless there is only one). */
    pickProgression() {
        const n = this.style.progressions.length;
        if (n <= 1)
            return 0;
        const cur = this.progIndex;
        const r = this.progRng.next() * (n + 1);
        let acc = 0;
        for (let k = 1; k < n; k++) {
            acc += k === 1 ? 3 : 1;
            if (r < acc)
                return (cur + k) % n;
        }
        return (cur + 1) % n;
    }
    /** One chord per bar (one per two bars in intro and post). */
    advanceChord(sec) {
        if (this.holdBars > 0) {
            this.holdBars--;
            return;
        }
        const prog = this.style.progressions[this.progIndex];
        if (this.progChord >= prog.length) {
            this.progIndex = this.pickProgression();
            this.progChord = 0;
        }
        this.chordDegree = this.style.progressions[this.progIndex][this.progChord++];
        this.holdBars = sec === 'intro' || sec === 'post' ? 1 : 0;
        this.voiceChord();
    }
    /** In-scale triad of the degree, folded into one octave, then the inversion nearest the
     *  previous voicing (voice leading). */
    voiceChord() {
        this.buildTriad(this.chordDegree, this.triad);
        const p = this.triad, v = this.padVoices, prev = this.prevPad;
        let best = Infinity;
        for (let inv = 0; inv < 3; inv++) {
            const a = inv === 0 ? p[0] : inv === 1 ? p[1] : p[2];
            const b = inv === 0 ? p[1] : inv === 1 ? p[2] : p[0] + 12;
            const c = inv === 0 ? p[2] : inv === 1 ? p[0] + 12 : p[1] + 12;
            const cost = this.hasPrevPad ? Math.abs(a - prev[0]) + Math.abs(b - prev[1]) + Math.abs(c - prev[2]) : inv;
            if (cost <= best) {
                best = cost;
                v[0] = a;
                v[1] = b;
                v[2] = c;
            }
        }
        prev[0] = v[0];
        prev[1] = v[1];
        prev[2] = v[2];
        this.hasPrevPad = true;
    }
    buildTriad(deg, out) {
        const sc = this.style.scale, root = this.style.root + this.transpose, L = sc.length;
        for (let k = 0; k < 3; k++) {
            const g = deg + 2 * k, oct = Math.floor(g / L);
            let pitch = root + oct * 12 + sc[g - oct * L];
            while (pitch >= root + 12)
                pitch -= 12;
            while (pitch < root)
                pitch += 12;
            out[k] = pitch;
        }
        if (out[0] > out[1]) {
            const t = out[0];
            out[0] = out[1];
            out[1] = t;
        }
        if (out[1] > out[2]) {
            const t = out[1];
            out[1] = out[2];
            out[2] = t;
        }
        if (out[0] > out[1]) {
            const t = out[0];
            out[0] = out[1];
            out[1] = t;
        }
    }
    scalePitch(g) {
        const sc = this.style.scale, L = sc.length, oct = Math.floor(g / L);
        return this.style.root + this.transpose + oct * 12 + sc[g - oct * L];
    }
    totalStep() { return this.bar * BAR + this.step; }
    /** Pad, bass and drums of the current step; the melodic layers come from emitMelodicLayers. */
    emitSection(out, n, g) {
        const s = this.sectionState, step = this.step;
        if (s === 'resolveWin')
            return step === 0 ? this.emitResolve(out, n, true) : n;
        if (s === 'resolve') {
            if (step === 0)
                n = this.emitResolve(out, n, false);
            // A slow falling pluck arpeggio of the tonic triad in quarters keeps the resolve moving.
            if ((step & 3) === 0)
                n = this.put(out, n, 'pluck', this.triad[2 - ((step >> 2) % 3)] + 12, 0.4, this.totalStep(), 4);
            return n;
        }
        if (s === 'post') {
            // Calm: the held chord over its bass root, and one soft pluck chord tone per half bar.
            const at = this.totalStep();
            if ((step & 7) === 0)
                n = this.put(out, n, 'pluck', this.padVoices[(step >> 3) + 1] + 12, 0.3, at, 8);
            if (step !== 0)
                return n;
            const vel = Math.min(0.4, 0.18 + 0.2 * this.style.palette.pad);
            for (let k = 0; k < 3; k++)
                n = this.put(out, n, 'pad', this.padVoices[k], vel, at, BAR);
            return this.put(out, n, 'bass', this.scalePitch(this.chordDegree) - 12, 0.25, at, BAR);
        }
        if (step === 0) {
            if (this.crashBar) {
                n = this.put(out, n, 'perc', CRASH, 0.85, this.totalStep(), 4);
                this.crashBar = false;
            }
            n = this.emitPad(out, n);
        }
        if (s === 'intro')
            return n;
        if (g.bass)
            n = this.emitBass(out, n, step);
        if (g.drums)
            n = this.emitDrums(out, n, step);
        return n;
    }
    emitPad(out, n) {
        const s = this.sectionState, p = this.style.palette.pad;
        const vel = s === 'post' ? Math.min(0.4, 0.18 + 0.22 * p) : 0.3 + 0.35 * p;
        const at = this.totalStep();
        for (let k = 0; k < 3; k++)
            n = this.put(out, n, 'pad', this.padVoices[k], vel, at, BAR);
        if (s === 'intro') {
            const riser = this.scalePitch(this.chordDegree + 4 + this.introBars);
            n = this.put(out, n, 'pad', riser, clamp(0.25 + 0.05 * this.introBars, 0, 0.7), at, BAR);
        }
        return n;
    }
    /** resolveWin: the in-scale major resolve (I, or the relative major III in minor and dorian), a
     *  crash and a held pad. resolve: the softer tonic resolve at velocity <= 0.5, no crash, with a
     *  quiet pluck arpeggio. Both chords and their bass root hold for the 2-bar section, into post. */
    emitResolve(out, n, win) {
        const deg = win && this.style.scale[2] < 4 ? 2 : 0;
        this.buildTriad(deg, this.triad);
        const at = this.totalStep(), vel = win ? 0.85 : 0.45, len = 2 * BAR;
        if (win)
            n = this.put(out, n, 'perc', CRASH, 0.9, at, 4);
        for (let k = 0; k < 3; k++)
            n = this.put(out, n, 'pad', this.triad[k], vel, at, len);
        n = this.put(out, n, 'bass', this.triad[0] - 12, win ? 0.6 : 0.45, at, len);
        return n;
    }
    /** Root and fifth; 8ths below intensity 0.6, with 16th syncopation above, an octave under the pad. */
    emitBass(out, n, step) {
        const root = this.scalePitch(this.chordDegree) - 12, fifth = this.scalePitch(this.chordDegree + 4) - 12;
        const vel = 0.45 + 0.3 * this.style.palette.bass, at = this.totalStep();
        if (step % 2 === 0)
            n = this.put(out, n, 'bass', step === 6 || step === 14 ? fifth : root, vel, at, 1);
        else if (this.state.intensity >= 0.6 && step % 4 === 3)
            n = this.put(out, n, 'bass', fifth, vel * 0.7, at, 1);
        return n;
    }
    /** Per-section beat with hats scaled by intensity; the fill bar before a section change plays
     *  toms and rising snare 16ths. climax and intense get the full pattern. */
    emitDrums(out, n, step) {
        const at = this.totalStep(), pal = this.style.palette;
        const hv = clamp(0.35 + 0.4 * pal.drums, 0, 0.9) * clamp(0.5 + 0.5 * this.state.intensity, 0, 1) * (1 - 0.08 * this.rng.next());
        const full = this.sectionState === 'climax' || this.sectionState === 'intense';
        if (this.fillBar) {
            if (step < 8) {
                if (step === 0)
                    n = this.put(out, n, 'kick', KICK, 0.8, at, 1);
                if (step === 4)
                    n = this.put(out, n, 'snare', SNARE, 0.7, at, 1);
                if (step % 2 === 0)
                    n = this.put(out, n, 'hat', HAT_C, hv, at, 1);
            }
            else {
                const rv = (step - 8) / 8;
                n = this.put(out, n, 'snare', SNARE, 0.3 + 0.55 * rv, at, 1);
                if (step % 2 === 0)
                    n = this.put(out, n, 'perc', FILL_TOMS[(step - 8) >> 1], 0.35 + 0.5 * rv, at, 1);
            }
            return n;
        }
        if (step === 0 || step === 8)
            n = this.put(out, n, 'kick', KICK, 0.85, at, 1);
        if (full && (step === 6 || step === 14))
            n = this.put(out, n, 'kick', KICK, 0.6, at, 1);
        if (step === 4 || step === 12)
            n = this.put(out, n, 'snare', SNARE, 0.75, at, 1);
        if (full && step === 15)
            n = this.put(out, n, 'snare', RIM, 0.5, at, 1);
        if (step % 2 === 0)
            n = this.put(out, n, 'hat', HAT_C, hv, at, 1);
        else if (full)
            n = this.put(out, n, 'hat', HAT_C, hv * 0.55, at, 1);
        if (full && step === 6)
            n = this.put(out, n, 'hat', HAT_O, hv, at, 1);
        if (pal.perc > 0.3 && step % 4 === 2)
            n = this.put(out, n, 'perc', SHAKER, 0.3 * pal.perc, at, 1);
        return n;
    }
    /** Writes pool[n] and returns n + 1; never allocates. */
    put(out, n, inst, midi, vel, step, len) {
        if (n >= POOL || !(vel > 0) || !Number.isFinite(midi))
            return n;
        const e = this.pool[n];
        e.inst = inst;
        e.midi = midi | 0;
        e.vel = clamp(vel, 0, 1);
        e.step = step;
        e.len = len;
        out[n] = e;
        return n + 1;
    }
    /** Intensity 0..1 to the active layers: pad always, then bass 0.2, drums 0.35, pluck 0.5,
     *  lead 0.65 and counter-melody 0.8. Returns the reused gate object. */
    gate(intensity) {
        const g = this.gateState;
        g.pad = true;
        g.bass = intensity >= 0.2;
        g.drums = intensity >= 0.35;
        g.pluck = intensity >= 0.5;
        g.lead = intensity >= 0.65;
        g.counter = intensity >= 0.8;
        return g;
    }
    /** Melodic layers: the motif lead, the counter-melody, the pluck arpeggio, the sparkle layer and
     *  the resolveWin flourish. intro, resolve and post emit no lead. */
    emitMelodicLayers(out, n, g) {
        const s = this.sectionState, step = this.step, at = this.totalStep();
        if (s === 'resolveWin' && this.bar === this.finishBar && step < 8 && (step & 1) === 0)
            n = this.emitFlourish(out, n, step >> 1, at);
        if (s !== 'intro' && s !== 'resolve' && s !== 'resolveWin' && s !== 'post') {
            if (step === 0)
                this.onMotifBar();
            const pos = ((this.bar - this.motifBar) & 1) * BAR + step;
            if (g.lead)
                n = this.emitLead(out, n, pos, at);
            if (g.counter)
                n = this.emitCounter(out, n, pos, at);
            if (g.pluck)
                n = this.emitArp(out, n, step, at);
        }
        if (this.state.sparkle > 0.5 && s !== 'intro' && s !== 'post')
            n = this.emitSparkle(out, n, step, at);
        return n;
    }
    /** A new motif on entering a section or after a style change; then one variation per 2-bar
     *  phrase, cycling original, transposition, displacement and inversion, so the 4th repetition
     *  always varies and a motif never repeats more than twice unchanged. */
    onMotifBar() {
        const sec = SECTIONS.indexOf(this.sectionState);
        if (sec !== this.motifSec || this.activeStyle !== this.motifStyle) {
            this.motifSec = sec;
            this.motifStyle = this.activeStyle;
            this.motifBar = this.bar;
            this.makeMotif(this.motifSeeds[sec * this.styles.length + this.activeStyle]);
        }
        const rel = this.bar - this.motifBar;
        if ((rel & 1) !== 0)
            return;
        const r = this.motifRng;
        this.varKind = (rel >> 1) & 3;
        this.varAmt = this.varKind === V_TRANSPOSE ? (r.next() < 0.5 ? -1 : 1) * r.int(1, 2)
            : this.varKind === V_DISPLACE ? r.int(1, 2) : 0;
    }
    /** 1 or 2 bars of 4-step rhythm cells; degrees relative to the chord root, stepwise motion
     *  preferred and leaps of at most a 5th. Legato notes overlap the next onset (lead glide). */
    makeMotif(seed) {
        const r = this.motifRng, L = this.style.scale.length, maxLeap = L >= 7 ? 4 : 3;
        r.state = seed;
        this.mSteps = r.next() < 0.5 ? BAR : 2 * BAR;
        this.mOn.fill(0);
        this.mCover.fill(0);
        let deg = r.next() < 0.5 ? 0 : 2, first = true;
        for (let c = 0; c < this.mSteps >> 2; c++) {
            const mask = CELLS[c === 0 ? r.int(0, 6) : r.int(0, CELLS.length - 1)];
            for (let k = 0; k < 4; k++) {
                if (((mask >> k) & 1) === 0)
                    continue;
                if (!first) {
                    const u = r.next(), mv = u < 0.6 ? 1 : u < 0.8 ? 2 : u < 0.9 ? 0 : r.int(3, maxLeap);
                    let dir = r.next() < 0.5 ? -1 : 1;
                    if (deg + dir * mv > L || deg + dir * mv < -2)
                        dir = -dir;
                    deg += dir * mv;
                }
                first = false;
                this.mOn[c * 4 + k] = 1;
                this.mDeg[c * 4 + k] = deg;
            }
        }
        for (let i = 0; i < this.mSteps; i++) {
            if (this.mOn[i] === 0)
                continue;
            let gap = 1;
            while (i + gap < this.mSteps && this.mOn[i + gap] === 0)
                gap++;
            const len = Math.min(8, r.next() < 0.5 ? gap + 1 : Math.max(1, gap >> 1));
            this.mLen[i] = len;
            for (let k = 0; k < len && i + k < this.mSteps; k++)
                this.mCover[i + k] = 1;
        }
    }
    motifIndex(pos) {
        const d = this.varKind === V_DISPLACE ? this.varAmt : 0;
        return (((pos - d) % this.mSteps) + this.mSteps) % this.mSteps;
    }
    /** Octave that puts the lead's root in 65..76. */
    leadShift() { return 12 * Math.floor((76 - this.style.root) / 12); }
    /** The motif as 'lead': an octave up in intense, doubled 12 semitones up at velocity × 0.7 in climax. */
    emitLead(out, n, pos, at) {
        const i = this.motifIndex(pos);
        if (this.mOn[i] === 0)
            return n;
        const L = this.style.scale.length;
        let deg = this.mDeg[i];
        if (this.varKind === V_TRANSPOSE)
            deg += this.varAmt;
        else if (this.varKind === V_INVERT)
            deg = -deg;
        let gd = this.chordDegree + deg;
        while (gd > L)
            gd -= L;
        while (gd < -2)
            gd += L;
        const midi = this.scalePitch(gd) + this.leadShift() + (this.sectionState === 'intense' ? 12 : 0);
        const vel = 0.55 + 0.3 * this.style.palette.lead, len = this.mLen[i];
        n = this.mel(out, n, 'lead', midi, vel, at, len);
        if (this.sectionState === 'climax')
            n = this.mel(out, n, 'lead', midi + 12, vel * 0.7, at, len);
        return n;
    }
    /** Counter-melody: descending chord tones in 8ths, an octave under the lead, in the motif's rests. */
    emitCounter(out, n, pos, at) {
        if ((this.step & 1) !== 0 || this.mCover[this.motifIndex(pos)] !== 0)
            return n;
        const midi = this.padVoices[2 - ((this.step >> 1) % 3)] + this.leadShift() - 12;
        return this.mel(out, n, 'pluck', midi, 0.35 + 0.25 * this.style.palette.lead, at, 2);
    }
    /** Chord tones up then down, an octave above the pad: 8ths, or 16ths in intense and climax. */
    emitArp(out, n, step, at) {
        const fast = this.sectionState === 'intense' || this.sectionState === 'climax';
        if (!fast && (step & 1) !== 0)
            return n;
        const i = this.arpIndex++ % 6, k = i < 4 ? i : 6 - i;
        const midi = (k === 3 ? this.padVoices[0] + 12 : this.padVoices[k]) + 12;
        return this.mel(out, n, 'pluck', midi, 0.3 + 0.35 * this.style.palette.pluck, at, fast ? 1 : 2);
    }
    /** sparkle: a bright 16th arpeggio of chord tones 2 octaves above the pad, velocity 0.5-0.7. */
    emitSparkle(out, n, step, at) {
        const k = step & 3, j = ((step >> 2) & 1) !== 0 ? 3 - k : k;
        const midi = (j === 3 ? this.padVoices[0] + 12 : this.padVoices[j]) + 24;
        return this.mel(out, n, 'pluck', midi, 0.5 + 0.2 * this.rng.next(), at, 1);
    }
    /** resolveWin: 4 ascending scale notes ending on the resolve chord's root (degree as emitResolve). */
    emitFlourish(out, n, i, at) {
        const deg = this.style.scale[2] < 4 ? 2 : 0;
        return this.mel(out, n, 'lead', this.scalePitch(deg - 3 + i) + this.leadShift(), 0.5 + 0.1 * i, at, i === 3 ? 12 : 2);
    }
    /** Scale quantizer of the melodic layers: snaps down to the nearest pitch of the transposed scale. */
    mel(out, n, inst, midi, vel, at, len) {
        const sc = this.style.scale, pc = (((midi - this.style.root - this.transpose) % 12) + 12) % 12;
        let k = sc.length - 1;
        while (k > 0 && sc[k] > pc)
            k--;
        return this.put(out, n, inst, midi - (pc - sc[k]), vel, at, len);
    }
}
