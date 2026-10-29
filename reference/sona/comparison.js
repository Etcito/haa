import { analyzePCM, clampRegion, computeLevelMatch, cropPCM } from './comparison-analysis.js';

const noop = () => {};
export const REFERENCE_LIMITS = Object.freeze({ maxFileBytes: 60 * 1024 * 1024, maxDurationSeconds: 180 });
const abortError = () => Object.assign(new Error('La préparation audio a été remplacée par une demande plus récente.'), { name: 'AbortError' });
const safeSide = side => {
  if (side !== 'reference' && side !== 'synthesis') throw new Error('Choisissez la référence ou la synthèse.');
  return side;
};

export class ReferenceComparator {
  constructor({ onTime = noop, onEnded = noop, onError = noop } = {}) {
    console.info('[SonaLab] Initialisation de la comparaison audio');
    this.onTime = onTime; this.onEnded = onEnded; this.onError = onError;
    this.context = null; this.masterNode = null;
    this._reference = null; this._synthesis = null; this._metadata = null;
    this._region = { start: 0, length: 0, end: 0, duration: 0 };
    this._prepared = null; this._session = null; this._timer = null;
    this._master = .65; this._matchLevels = true; this._side = 'reference'; this._isPlaying = false;
    this._loadSerial = 0; this._renderSerial = 0; this._playSerial = 0;
    this._fetchController = null; this._renderPromise = null; this._pendingGraphKey = null; this._graphKey = null;
  }

  get isPlaying() { return this._isPlaying; }
  get hasReference() { return !!this._reference; }
  get referenceMetadata() { return this._metadata ? { ...this._metadata } : null; }
  get region() { return { ...this._region }; }
  get playbackDuration() { return this._reference ? Math.min(this._region.length, this._synthesis?.duration ?? Infinity) : 0; }

  _ensureContext() {
    if (!this.context || this.context.state === 'closed') {
      const Context = globalThis.AudioContext || globalThis.webkitAudioContext;
      if (!Context) throw new Error('Ce navigateur ne prend pas en charge Web Audio.');
      this.context = new Context({ latencyHint: 'interactive' });
      this.masterNode = this.context.createGain();
      this.masterNode.gain.value = this._master; this.masterNode.connect(this.context.destination);
    }
    return this.context;
  }

  _report(error) {
    if (error.name !== 'AbortError') this.onError(error);
    return error;
  }

  _beginLoad() {
    this._fetchController?.abort(); this._fetchController = null;
    this.stop();
    return ++this._loadSerial;
  }

  async _decodeReference(bytes, name, serial) {
    if (serial !== this._loadSerial) throw abortError();
    if (!(bytes instanceof ArrayBuffer) || !bytes.byteLength) throw new Error('Le fichier audio est vide ou illisible.');
    if (bytes.byteLength > REFERENCE_LIMITS.maxFileBytes) throw new Error('Le fichier dépasse 60 Mo. Importez un extrait plus court.');
    let buffer;
    try { buffer = await this._ensureContext().decodeAudioData(bytes.slice(0)); }
    catch (error) {
      if (serial !== this._loadSerial) throw abortError();
      throw new Error('Ce fichier n’a pas pu être décodé. Importez un fichier WAV, MP3, OGG ou M4A pris en charge par votre navigateur.', { cause: error });
    }
    if (serial !== this._loadSerial) throw abortError();
    if (!buffer.length || !buffer.duration) throw new Error('Le fichier audio ne contient aucun échantillon.');
    if (buffer.duration > REFERENCE_LIMITS.maxDurationSeconds) throw new Error('La référence dépasse 180 secondes. Découpez un extrait avant de l’importer.');
    this._reference = buffer;
    this._metadata = { name: String(name || 'Référence audio'), duration: buffer.duration, sampleRate: buffer.sampleRate, channels: buffer.numberOfChannels };
    this._region = clampRegion(buffer.duration, 0, Math.min(30, buffer.duration));
    this._prepared = null;
    console.info('[SonaLab] Référence audio décodée', { duration: buffer.duration, channels: buffer.numberOfChannels });
    return this.referenceMetadata;
  }

  async loadFile(file) {
    const serial = this._beginLoad();
    try {
      if (!file || typeof file.arrayBuffer !== 'function') throw new Error('Sélectionnez un fichier audio à importer.');
      if (file.size > REFERENCE_LIMITS.maxFileBytes) throw new Error('Le fichier dépasse 60 Mo. Importez un extrait plus court.');
      return await this._decodeReference(await file.arrayBuffer(), file.name, serial);
    } catch (error) { throw this._report(serial === this._loadSerial ? error : abortError()); }
  }

  async loadUrl(url, name) {
    const serial = this._beginLoad();
    const controller = new AbortController(); this._fetchController = controller;
    try {
      let response;
      try { response = await fetch(url, { mode: 'cors', credentials: 'same-origin', signal: controller.signal }); }
      catch (error) {
        if (serial !== this._loadSerial || error.name === 'AbortError') throw abortError();
        throw new Error('Le fichier audio est inaccessible depuis ce navigateur (adresse indisponible ou accès CORS refusé). Téléchargez-le puis importez-le dans le comparateur.', { cause: error });
      }
      if (!response.ok) throw new Error(`Le téléchargement du son a échoué (HTTP ${response.status}). Vous pouvez importer le fichier audio directement.`);
      if (Number(response.headers.get('content-length')) > REFERENCE_LIMITS.maxFileBytes) throw new Error('Le fichier dépasse 60 Mo. Importez un extrait plus court.');
      if (/text\/html/i.test(response.headers.get('content-type') || '')) throw new Error('Ce lien ouvre une page web. Téléchargez le fichier audio sur cette page, puis importez-le ici.');
      const inferredName = String(url).split('/').pop()?.split('?')[0] || 'Référence audio';
      return await this._decodeReference(await response.arrayBuffer(), name || inferredName, serial);
    } catch (error) { throw this._report(serial === this._loadSerial ? error : abortError()); }
    finally { if (this._fetchController === controller) this._fetchController = null; }
  }

  async setReferenceBuffer(arrayBuffer, name = 'Référence audio') {
    const serial = this._beginLoad();
    try { return await this._decodeReference(arrayBuffer, name, serial); }
    catch (error) { throw this._report(serial === this._loadSerial ? error : abortError()); }
  }

  setRegion(startSeconds, lengthSeconds) {
    if (!this._reference) throw new Error('Importez d’abord un son de référence.');
    const region = clampRegion(this._reference.duration, startSeconds, lengthSeconds);
    if (region.start === this._region.start && region.length === this._region.length) return this.region;
    this.stop(); this._region = region; this._prepared = null;
    return this.region;
  }

  clearReference() {
    this.stop(); this._loadSerial++; this._fetchController?.abort(); this._fetchController = null;
    this._reference = null; this._metadata = null; this._prepared = null;
    this._region = { start: 0, length: 0, end: 0, duration: 0 };
  }

  async prepareSynthesis(graph, audioEngine) {
    if (!audioEngine || typeof audioEngine.renderWav !== 'function') throw new Error('Le moteur de synthèse est indisponible.');
    const graphKey = JSON.stringify(graph);
    if (this._synthesis && graphKey === this._graphKey) {
      // Returning to the cached graph also invalidates a different pending render.
      if (this._pendingGraphKey && this._pendingGraphKey !== graphKey) {
        this._renderSerial++; this._pendingGraphKey = null; this._renderPromise = null;
      }
      return this.getAnalysis();
    }
    if (this._renderPromise && graphKey === this._pendingGraphKey) return this._renderPromise;
    this.stop();
    const serial = ++this._renderSerial;
    const snapshot = JSON.parse(graphKey);
    this._pendingGraphKey = graphKey;
    console.info('[SonaLab] Rendu de la synthèse pour comparaison', { duration: snapshot.duration });
    const task = (async () => {
      try {
        const wav = await audioEngine.renderWav(snapshot, { duration: snapshot.duration, master: 1 });
        if (serial !== this._renderSerial) throw abortError();
        const bytes = wav instanceof ArrayBuffer ? wav : await wav.arrayBuffer();
        if (serial !== this._renderSerial) throw abortError();
        const buffer = await this._ensureContext().decodeAudioData(bytes.slice(0));
        if (serial !== this._renderSerial) throw abortError();
        this._synthesis = buffer; this._graphKey = graphKey; this._prepared = null;
        return this.getAnalysis();
      } catch (error) { throw this._report(serial === this._renderSerial ? error : abortError()); }
      finally {
        if (serial === this._renderSerial) { this._renderPromise = null; this._pendingGraphKey = null; }
      }
    })();
    this._renderPromise = task;
    return task;
  }

  invalidateSynthesis() {
    this.stop(); this._renderSerial++; this._synthesis = null; this._graphKey = null;
    this._pendingGraphKey = null; this._renderPromise = null; this._prepared = null;
  }

  _makeSegment(buffer, start, duration) {
    const source = Array.from({ length: buffer.numberOfChannels }, (_, index) => buffer.getChannelData(index));
    const channels = cropPCM(source, buffer.sampleRate, start, duration);
    const segment = this._ensureContext().createBuffer(channels.length, channels[0].length, buffer.sampleRate);
    for (let index = 0; index < channels.length; index++) segment.copyToChannel(channels[index], index);
    return { buffer: segment, analysis: analyzePCM(channels, buffer.sampleRate) };
  }

  _prepareSegments() {
    if (this._prepared || !this._reference) return this._prepared;
    const duration = this.playbackDuration;
    const reference = this._makeSegment(this._reference, this._region.start, duration);
    const synthesis = this._synthesis ? this._makeSegment(this._synthesis, 0, duration) : null;
    this._prepared = { reference, synthesis, duration: Math.min(reference.buffer.duration, synthesis?.buffer.duration ?? Infinity) };
    return this._prepared;
  }

  getAnalysis() {
    const prepared = this._prepareSegments();
    const reference = prepared?.reference.analysis ?? null, synthesis = prepared?.synthesis?.analysis ?? null;
    const levels = computeLevelMatch(reference, synthesis, this._matchLevels);
    return {
      reference, synthesis, ...levels,
      gainReference: levels.referenceGain, gainSynthesis: levels.synthesisGain,
      region: this.region, duration: prepared?.duration ?? 0, playbackDuration: prepared?.duration ?? 0,
      edgeFadeSeconds: .005, matchLevels: this._matchLevels,
    };
  }

  getReferencePCM({ regionOnly = false } = {}) {
    if (!this._reference) return null;
    const buffer = regionOnly ? this._prepareSegments().reference.buffer : this._reference;
    return {
      channels: Array.from({ length: buffer.numberOfChannels }, (_, index) => buffer.getChannelData(index).slice()),
      sampleRate: buffer.sampleRate, duration: buffer.duration,
    };
  }

  async play({ side = 'reference', loop = true, master = this._master, matchLevels = this._matchLevels, offset = 0 } = {}) {
    this.stop();
    const serial = this._playSerial;
    try {
      this._side = safeSide(side);
      if (!this._reference) throw new Error('Importez un son de référence avant de lancer la comparaison.');
      if (!this._synthesis) throw new Error('Préparez la synthèse avant de lancer la comparaison.');
      this.setMaster(master); this.setMatchLevels(matchLevels);
      const context = this._ensureContext();
      await context.resume();
      if (serial !== this._playSerial) return false;
      const prepared = this._prepareSegments(), analysis = this.getAnalysis();
      const duration = prepared.duration;
      const position = Math.max(0, Math.min(duration - 1 / context.sampleRate, Number(offset) || 0));
      const when = context.currentTime + .025;
      const session = { sources: {}, sideGains: {}, levelGains: {}, when, offset: position, loop: !!loop, duration, serial, transition: null, output: context.createGain() };
      session.output.gain.setValueAtTime(0, when);
      session.output.gain.linearRampToValueAtTime(1, when + .005);
      session.output.connect(this.masterNode);
      this._session = session;
      for (const name of ['reference', 'synthesis']) {
        const source = context.createBufferSource(), sideGain = context.createGain(), levelGain = context.createGain();
        source.buffer = prepared[name].buffer; source.loop = !!loop; source.loopStart = 0; source.loopEnd = duration;
        sideGain.gain.value = name === this._side ? 1 : 0;
        levelGain.gain.value = name === 'reference' ? analysis.gainReference : analysis.gainSynthesis;
        source.connect(levelGain); levelGain.connect(sideGain); sideGain.connect(session.output);
        session.sources[name] = source; session.sideGains[name] = sideGain; session.levelGains[name] = levelGain;
      }
      session.sources.reference.onended = () => {
        if (this._session !== session || serial !== this._playSerial || session.loop) return;
        this._isPlaying = false; this._session = null; clearInterval(this._timer); this._timer = null;
        this._disconnect(session); this.onTime(duration); this.onEnded();
      };
      for (const source of Object.values(session.sources)) {
        source.start(when, position);
        if (!loop) source.stop(when + duration - position);
      }
      this._isPlaying = true;
      this.onTime(position);
      this._timer = setInterval(() => {
        if (this._session !== session) return;
        const elapsed = Math.max(position, context.currentTime - when + position);
        this.onTime(session.loop ? elapsed % duration : Math.min(duration, elapsed));
      }, 40);
      console.info('[SonaLab] Comparaison A/B en lecture', { side, duration, matchLevels: analysis.matching });
      return true;
    } catch (error) {
      if (serial !== this._playSerial) throw abortError();
      this.stop(); throw this._report(error);
    }
  }

  switchSide(side) {
    const next = safeSide(side);
    const session = this._session;
    if (!session) { this._side = next; return; }
    if (next === this._side) return;
    const now = this.context.currentTime, transition = session.transition;
    let referenceGain = this._side === 'reference' ? 1 : 0;
    if (transition) {
      const fraction = Math.min(1, Math.max(0, (now - transition.start) / .02));
      referenceGain = transition.from + (transition.to - transition.from) * fraction;
    }
    const target = next === 'reference' ? 1 : 0;
    for (const name of ['reference', 'synthesis']) {
      const parameter = session.sideGains[name].gain;
      parameter.cancelScheduledValues(now);
      parameter.setValueAtTime(name === 'reference' ? referenceGain : 1 - referenceGain, now);
      parameter.linearRampToValueAtTime(name === 'reference' ? target : 1 - target, now + .02);
    }
    session.transition = { from: referenceGain, to: target, start: now };
    this._side = next;
  }

  setMaster(value) {
    this._master = Math.min(1, Math.max(0, Number(value) || 0));
    if (this.masterNode && this.context.state !== 'closed') this.masterNode.gain.setTargetAtTime(this._master, this.context.currentTime, .012);
    return this._master;
  }

  setMatchLevels(enabled) {
    this._matchLevels = !!enabled;
    if (this._session) {
      const analysis = this.getAnalysis(), now = this.context.currentTime;
      this._session.levelGains.reference.gain.setTargetAtTime(analysis.gainReference, now, .01);
      this._session.levelGains.synthesis.gain.setTargetAtTime(analysis.gainSynthesis, now, .01);
    }
  }

  _disconnect(session) {
    for (const source of Object.values(session.sources)) { source.onended = null; try { source.disconnect(); } catch {} }
    for (const gain of [...Object.values(session.sideGains), ...Object.values(session.levelGains), session.output]) { try { gain.disconnect(); } catch {} }
  }

  stop() {
    this._playSerial++; this._isPlaying = false;
    clearInterval(this._timer); this._timer = null;
    const session = this._session; this._session = null;
    if (!session) return;
    const now = this.context.currentTime;
    session.output.gain.cancelScheduledValues(now);
    session.output.gain.setValueAtTime(session.output.gain.value, now);
    session.output.gain.linearRampToValueAtTime(0, now + .005);
    for (const source of Object.values(session.sources)) {
      source.onended = null;
      try { source.stop(now + .006); } catch {}
    }
    setTimeout(() => this._disconnect(session), 35);
    console.info('[SonaLab] Comparaison audio arrêtée');
  }

  async dispose() {
    this.stop(); this._loadSerial++; this._renderSerial++; this._fetchController?.abort();
    this._reference = null; this._synthesis = null; this._prepared = null; this._metadata = null;
    if (this.context && this.context.state !== 'closed') await this.context.close();
    this.context = null; this.masterNode = null;
  }
}
