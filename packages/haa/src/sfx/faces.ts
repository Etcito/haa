// Distributed sources on the level's faces (docs/physics-sound.md, "Rain on faces"): rain is not
// one loop around the listener but one emitter per exposed face, each fed by that face's geometry
// (normal against the rain direction, exposed area), its material and its hand-chosen body. The
// emitters go through the normal spatial chain: inside the hall the rain on its roof is above you,
// coloured by the room; through the door, the rain on the ground outside.
import type { RayCast, Vec3 } from './acoustics.ts';
import { add, dot, length, normalize, scale, sub, v3 } from './acoustics.ts';
import type { ResonatorShape, SfxInputs, SoundMaterial } from './contract.ts';
import { areaFromSquareMetres, bodyInputs, materialInput } from './contract.ts';

/** A rectangular face of the level: centre, outward normal, half-extent vectors in its plane. */
export interface SoundFace {
  center: Vec3;
  normal: Vec3;
  /** Half extents: the face spans center ± u ± v. */
  u: Vec3;
  v: Vec3;
  /** Its material, with the object's body (`withBody`): hardness and resonator reach the preset. */
  material: SoundMaterial;
  /** Thickness (m) of the solid behind it: from the other side the emitter is on its far surface. */
  thickness?: number;
  label?: string;
}

export interface FaceSourceOptions {
  /** Preset played per face (e.g. `rain-on-face`): it reads `energy` (intensity), `flux`, `area`, `hardness`, `shape`, `dim`, `thickness`. */
  preset: string;
  /** Direction the rain falls (unit vector; (0, -1, 0) is straight down). */
  direction: Vec3;
  /** 0..1, passed as `energy`. */
  intensity: number;
  /** Faces playing at once, the loudest at the listener. Default 6. */
  maxVoices?: number;
  /** Exposure grid (m). Default 2. */
  cell?: number;
  gain?: number;
}

export interface FaceEmitter {
  face: SoundFace;
  /** max(0, −n·d). */
  flux: number;
  /** Exposed area (m²): the grid cells whose ray toward the rain escapes. */
  area: number;
  /** Where it plays from now, and from which side of the face (`under`: 0 its exposed side, 1 through it). */
  position: Vec3;
  back: boolean;
  under: number;
  /** Estimated level at the listener (for the choice of the playing faces). */
  score: number;
  /** 0..1, the fade of its voice. */
  fade: number;
  playing: boolean;
}

/** Loss (dB at 250 Hz, 1 kHz, 4 kHz) of a face heard from its far side, by body: a thin sheet radiates both ways, a slab barely. */
export const FAR_SIDE_LOSS_DB: Record<ResonatorShape, readonly [number, number, number]> = {
  sheet: [3, 5, 8],
  plate: [10, 16, 24],
  box: [8, 12, 18],
  shell: [6, 9, 14],
  bottle: [8, 12, 18],
  bar: [12, 18, 26],
  block: [28, 38, 50],
};

interface Sample {
  p: Vec3;
  exposed: boolean;
}

interface FaceState extends FaceEmitter {
  samples: Sample[];
  /** Cells along u and v (samples[i * nv + j]). */
  nu: number;
  nv: number;
  /** The listener's projection on the face, rounded (0.5 m), and whether the rain reaches it. */
  projKey: string;
  projExposed: boolean;
  cellArea: number;
  loop: ReturnType<FaceHost['loop']> | null;
}

/** What a face source needs from the engine. */
export interface FaceHost {
  raycast(): RayCast | undefined;
  listener(): Vec3;
  loop(preset: string, position: Vec3, inputs: Partial<SfxInputs>, o: { gain: number; extent: number; nearest: boolean; label: string }): {
    readonly alive: boolean;
    set(inputs: Partial<SfxInputs>): void;
    move(p: Vec3): void;
    setGain(g: number): void;
    setTransmission(db: readonly [number, number, number]): void;
    stop(fadeS?: number): void;
  };
}

export interface FaceSource {
  set(o: Partial<Pick<FaceSourceOptions, 'direction' | 'intensity' | 'gain'>>): void;
  /** Replace the faces (a roof changed material). */
  setFaces(faces: SoundFace[]): void;
  readonly emitters: readonly FaceEmitter[];
  tick(dt: number): void;
  stop(fadeS?: number): void;
}

const FADE_S = 0.6;
/** A playing face is replaced only by one estimated this much louder (dB): no flapping. */
const SWAP_MARGIN_DB = 3;

export function createFaceSource(host: FaceHost, faces: SoundFace[], options: FaceSourceOptions): FaceSource {
  let o = { maxVoices: 6, cell: 2, gain: 1, ...options };
  let states: FaceState[] = [];
  let stopped = false;
  let dirty = true;

  function build(list: SoundFace[]): void {
    for (const s of states) s.loop?.stop(0.3);
    states = list.map((face) => {
      const lu = length(face.u);
      const lv = length(face.v);
      const nu = Math.max(1, Math.min(40, Math.round((2 * lu) / o.cell)));
      const nv = Math.max(1, Math.min(40, Math.round((2 * lv) / o.cell)));
      const samples: Sample[] = [];
      for (let i = 0; i < nu; i++) {
        for (let j = 0; j < nv; j++) {
          const a = ((i + 0.5) / nu) * 2 - 1;
          const b = ((j + 0.5) / nv) * 2 - 1;
          samples.push({ p: add(face.center, add(scale(face.u, a), scale(face.v, b))), exposed: false });
        }
      }
      return {
        face, flux: 0, area: 0, position: face.center, back: false, under: 0, score: 0, fade: 0, playing: false,
        samples, nu, nv, projKey: '', projExposed: false, cellArea: (4 * lu * lv) / samples.length, loop: null,
      };
    });
    dirty = true;
  }

  /** Exposure: a ray from each cell toward where the rain comes from must escape. */
  function expose(): void {
    const cast = host.raycast();
    const up = scale(normalize(o.direction), -1);
    for (const s of states) {
      const n = normalize(s.face.normal);
      s.flux = Math.max(0, dot(n, up));
      s.projKey = '';
      let exposed = 0;
      for (const c of s.samples) {
        c.exposed = s.flux > 0 && (!cast || !cast(add(c.p, scale(n, 0.05)), up, 60));
        if (c.exposed) exposed++;
      }
      s.area = exposed * s.cellArea;
    }
    dirty = false;
  }

  function inputsOf(s: FaceState): Partial<SfxInputs> {
    return {
      energy: o.intensity,
      flux: s.flux,
      area: areaFromSquareMetres(s.area),
      hardness: s.face.material.hardness,
      ...bodyInputs(s.face.material.body),
      ...(s.face.material.material ? { material: materialInput(s.face.material.material) } : {}),
    };
  }

  function tick(dt: number): void {
    if (stopped) return;
    if (dirty) expose();
    const L = host.listener();
    for (const s of states) {
      if (!(s.flux > 0 && s.area > 0)) {
        s.score = 0;
        continue;
      }
      // The nearest exposed part: the nearest exposed cell, refined by the exposed cells around it
      // (within 2.5 cells, weighted by 1/(d² + 1)²) so the point glides as the listener walks. A
      // plain weighted mean of every exposed cell put the ground's emitter in the middle of the
      // hall, inside the ring of wet ground around it.
      let near: Sample | null = null;
      let best = Infinity;
      for (const c of s.samples) {
        if (!c.exposed) continue;
        const d2 = dot(sub(c.p, L), sub(c.p, L));
        if (d2 < best) {
          best = d2;
          near = c;
        }
      }
      if (!near) {
        s.score = 0;
        continue;
      }
      const reach2 = (2.5 * o.cell) ** 2;
      let wx = 0;
      let wy = 0;
      let wz = 0;
      let w = 0;
      for (const c of s.samples) {
        if (!c.exposed || dot(sub(c.p, near.p), sub(c.p, near.p)) > reach2) continue;
        const d2 = dot(sub(c.p, L), sub(c.p, L));
        const k = 1 / (d2 + 1) ** 2;
        wx += c.p.x * k;
        wy += c.p.y * k;
        wz += c.p.z * k;
        w += k;
      }
      const n = normalize(s.face.normal);
      let p = v3(wx / w, wy / w, wz / w);
      // Over the face (the rain reaches the listener's projection on it), the nearest point is
      // that projection: standing on wet ground the rain is right under the feet. The nearest cell
      // centre alone put it up to half a cell away (4 m cells on the 160 m ground: the ground's rain
      // 4.3 m away while a 6 m plank next to it, 2 m cells, played at 1.5 m: 9 dB louder).
      {
        const lu = length(s.face.u);
        const lv = length(s.face.v);
        const rel = sub(L, s.face.center);
        const a = Math.max(-1, Math.min(1, dot(rel, s.face.u) / (lu * lu)));
        const b = Math.max(-1, Math.min(1, dot(rel, s.face.v) / (lv * lv)));
        const q = add(s.face.center, add(scale(s.face.u, a), scale(s.face.v, b)));
        // Its own exposure ray (the cells are up to 4 m: a cell centre under the plank said the ground
        // beside it was dry), cast again only when the projection moved by half a metre.
        const key = `${Math.round(q.x * 2)},${Math.round(q.y * 2)},${Math.round(q.z * 2)}`;
        if (key !== s.projKey) {
          s.projKey = key;
          const cast = host.raycast();
          s.projExposed = s.flux > 0 && (!cast || !cast(add(q, scale(n, 0.05)), scale(normalize(o.direction), -1), 60));
        }
        if (s.projExposed) p = q;
      }
      // Under a roof (behind the face and within its footprint) the face radiates from the solid's
      // other surface, through it. The share `under` ramps over the last metre of the footprint, so
      // walking in under the roof edge moves the emitter and its loss gradually.
      const rel = sub(L, s.face.center);
      const lu = length(s.face.u);
      const lv = length(s.face.v);
      const inside = Math.min(lu - Math.abs(dot(rel, s.face.u)) / lu, lv - Math.abs(dot(rel, s.face.v)) / lv);
      const behind = dot(rel, n) < 0 && (s.face.thickness ?? 0) > 0;
      s.under = behind ? Math.min(1, Math.max(0, inside)) : 0;
      s.back = s.under > 0.5;
      const top = add(p, scale(n, 0.05));
      const below = sub(p, scale(n, (s.face.thickness ?? 0) + 0.05));
      s.position = add(scale(top, 1 - s.under), scale(below, s.under));
      const far = FAR_SIDE_LOSS_DB[s.face.material.body?.shape ?? 'block'];
      const loss = [far[0] * s.under, far[1] * s.under, far[2] * s.under] as const;
      const d = Math.max(1, length(sub(p, L)));
      s.score = (o.intensity ** 1.3 * s.flux * Math.sqrt(Math.min(25, s.area)) * (0.08 + 0.92 * s.face.material.hardness ** 1.5) * 10 ** (-loss[1] / 10)) / (d * d);
    }
    // The loudest faces play; a playing one leaves only for one clearly louder.
    const ranked = states.filter((s) => s.score > 0).sort((a, b) => b.score - a.score);
    const want = new Set(ranked.slice(0, o.maxVoices));
    for (const s of ranked.slice(o.maxVoices)) {
      if (!s.playing) continue;
      const weakest = [...want].filter((x) => !x.playing).sort((a, b) => a.score - b.score)[0];
      if (weakest && 10 * Math.log10(weakest.score / s.score) < SWAP_MARGIN_DB) {
        want.delete(weakest);
        want.add(s);
      }
    }
    for (const s of states) {
      const on = want.has(s);
      s.fade = Math.min(1, Math.max(0, s.fade + (on ? 1 : -1) * (dt / FADE_S)));
      const extent = Math.min(20, Math.sqrt(s.area / Math.PI));
      if (s.fade > 0 && !s.loop) {
        s.loop = host.loop(o.preset, s.position, inputsOf(s), { gain: 0, extent, nearest: true, label: `rain ${s.face.label ?? ''}`.trim() });
      }
      if (!s.loop) continue;
      if (s.fade <= 0) {
        s.loop.stop(0.05);
        s.loop = null;
        s.playing = false;
        continue;
      }
      s.playing = true;
      s.loop.move(s.position);
      s.loop.set(inputsOf(s));
      s.loop.setGain(o.gain * s.fade);
      const far = FAR_SIDE_LOSS_DB[s.face.material.body?.shape ?? 'block'];
      s.loop.setTransmission([far[0] * s.under, far[1] * s.under, far[2] * s.under]);
    }
  }

  build(faces);
  return {
    set(next) {
      const turned = next.direction && (next.direction.x !== o.direction.x || next.direction.y !== o.direction.y || next.direction.z !== o.direction.z);
      o = { ...o, ...next };
      if (turned) dirty = true;
    },
    setFaces(list) {
      build(list);
    },
    get emitters() {
      return states;
    },
    tick,
    stop(fadeS = 0.4) {
      stopped = true;
      for (const s of states) s.loop?.stop(fadeS);
    },
  };
}
