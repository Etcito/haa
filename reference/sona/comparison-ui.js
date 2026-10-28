import { t, tr, tx, html as ui, locale, registerStaticTranslations, translateError } from './i18n.js';
import { ReferenceComparator } from './comparison.js';
import { REFERENCES } from './reference-presets.js';

const $ = id => document.getElementById(id);
const esc = value => String(value ?? '').replace(/[&<>"']/g, char => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
const fixed = (value, digits = 2) => Number(value).toLocaleString(locale(), { maximumFractionDigits: digits, minimumFractionDigits: digits });
const clamp = (value, low, high) => Math.max(low, Math.min(high, value));

function openStore() {
  console.info('[Sona] Open reference storage');
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('sona-references', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('audio');
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
  });
}

async function savedReference(key, value) {
  console.info('[Sona] Read or save reference', key);
  const db = await openStore();
  return new Promise((resolve, reject) => {
    const write = arguments.length > 1;
    const transaction = db.transaction('audio', write ? 'readwrite' : 'readonly');
    const objectStore = transaction.objectStore('audio');
    const request = write ? value === null ? objectStore.delete(key) : objectStore.put(value, key) : objectStore.get(key);
    let result;
    request.onsuccess = () => { result = request.result; };
    transaction.oncomplete = () => { db.close(); resolve(result); };
    transaction.onerror = () => { db.close(); reject(transaction.error); };
    transaction.onabort = () => { db.close(); reject(transaction.error || new Error(tx('Enregistrement interrompu.'))); };
  });
}

export class ComparisonPanel {
  constructor({ audioEngine, getProject, beforePlay, onError }) {
    console.info('[Sona] Initialize comparison interface');
    this.audioEngine = audioEngine; this.getProject = getProject;
    this.beforePlay = beforePlay; this.onError = onError;
    this.entries = new Map(); this.key = null; this.epoch = 0;
    this.busy = false; this.side = 'reference'; this.position = 0; this.listenSerial = 0;
    this.mount(); this.bind();
  }

  mount() {
    console.info('[Sona] Mount comparison interface');
    document.body.insertAdjacentHTML('beforeend', ui`
      <dialog id="comparisonDialog" class="comparison-dialog" aria-labelledby="comparisonTitle">
        <header class="comparison-header">
          <div><span class="section-eyebrow">ÉCOUTE COMPARÉE</span><h2 id="comparisonTitle">Du réel au procédural<span>.</span></h2></div>
          <button id="closeComparison" class="icon-button" aria-label="Fermer le comparateur" type="button">×</button>
        </header>
        <div class="comparison-content">
          <section class="reference-card" aria-label="Son de référence">
            <div class="reference-mark">A</div>
            <div class="reference-details"><span class="section-eyebrow">ENREGISTREMENT DE RÉFÉRENCE</span><h3 id="referenceName">—</h3><p id="referenceMetadata">—</p></div>
            <div class="reference-actions"><a id="referenceSource" target="_blank" rel="noopener noreferrer">Source Pixabay ↗</a><button id="importReference" class="secondary-button" type="button">Importer un son</button><button id="resetReference" class="reference-reset" type="button" hidden>Revenir à la référence</button></div>
            <input id="referenceFile" type="file" accept="audio/*,.wav,.mp3,.m4a,.ogg,.flac" hidden>
          </section>
          <p id="comparisonNote" class="comparison-note"></p>
          <div id="comparisonMessage" class="comparison-message" role="status" aria-live="polite"></div>
          <section class="comparison-region" aria-label="Passage à comparer">
            <div><label for="referenceStart">Début de l’extrait</label><span><input id="referenceStart" type="number" min="0" step="0.01" value="0"> s</span></div>
            <div><label for="referenceLength">Durée de l’extrait</label><span><input id="referenceLength" type="number" min="0.25" max="30" step="0.01" value="1.68"> s</span></div>
            <p id="comparisonDuration">La synthèse commence au début du graphe.</p>
            <button id="prepareComparison" class="secondary-button" type="button">Actualiser la synthèse</button>
          </section>
          <section class="comparison-signals" aria-label="Écoute et formes d’onde">
            <div class="comparison-signal reference-signal">
              <button id="listenReference" class="comparison-listen" type="button" aria-pressed="false"><span class="comparison-letter">A</span><span><strong>Référence</strong><small>Enregistrement</small></span><i aria-hidden="true">▶</i></button>
              <div class="comparison-wave-wrap"><canvas id="referenceWave" role="img" aria-label="Forme d’onde de la référence"></canvas><span class="comparison-playhead"></span></div>
              <output id="referenceRms" class="comparison-rms">—<small>RMS</small></output>
            </div>
            <div class="comparison-signal synthesis-signal">
              <button id="listenSynthesis" class="comparison-listen" type="button" aria-pressed="false"><span class="comparison-letter">B</span><span><strong>Synthèse</strong><small>Votre graphe</small></span><i aria-hidden="true">▶</i></button>
              <div class="comparison-wave-wrap"><canvas id="synthesisWave" role="img" aria-label="Forme d’onde de la synthèse"></canvas><span class="comparison-playhead"></span></div>
              <output id="synthesisRms" class="comparison-rms">—<small>RMS</small></output>
            </div>
          </section>
          <div class="comparison-transport">
            <button id="stopComparison" class="secondary-button" type="button">■ Arrêter</button><span id="comparisonTime" class="comparison-time">0,00 s</span>
            <label><input id="comparisonLoop" type="checkbox" checked> Boucler</label>
            <label class="comparison-matching"><input id="matchReferenceLevels" type="checkbox" checked> Égaliser les niveaux <span title="${tr('Constant gain measured between 20 Hz and 20 kHz, excluding infrasonic rumble. Playback stays unfiltered. This is not a LUFS measurement.')}">RMS 20 Hz–20 kHz</span></label>
          </div>
          <section class="comparison-analysis" aria-label="Analyse des deux sons">
            <div class="comparison-analysis-heading"><h3>Équilibre spectral</h3><div><span class="reference-key">● Référence</span><span class="synthesis-key">● Synthèse</span></div></div>
            <canvas id="comparisonSpectrum" role="img" aria-label="Spectres moyens superposés de la référence et de la synthèse"></canvas>
            <div id="comparisonBands" class="comparison-bands"></div>
            <p class="comparison-analysis-note">Spectre moyen du passage · 20 Hz–20 kHz · gain de comparaison appliqué. Ces mesures aident à régler le timbre ; elles ne mesurent pas le réalisme.</p>
          </section>
        </div>
        <footer class="comparison-footer"><span id="referenceCredit">Les fichiers importés restent dans ce navigateur.</span><span>Raccourcis <kbd>A</kbd> / <kbd>B</kbd> · <kbd>Espace</kbd> arrêt</span></footer>
      </dialog>`);
    registerStaticTranslations($('comparisonDialog'));
  }

  entry() {
    const key = this.getProject().presetId || 'custom';
    if (this.key !== key) { this.stop(); this.key = key; this.epoch++; }
    if (!this.entries.has(key)) {
      const item = { custom: false, initialized: false, dirty: true, error: '', loading: null };
      item.engine = new ReferenceComparator({
        onTime: seconds => { if (this.key === key) this.updatePosition(seconds); },
        onEnded: () => { if (this.key === key) this.updatePlayback(); },
      });
      this.entries.set(key, item);
    }
    return this.entries.get(key);
  }

  async open() {
    if ($('comparisonDialog').open) return;
    console.info('[Sona] Ouverture du comparateur');
    this.beforePlay();
    $('comparisonDialog').showModal();
    $('comparisonDialog').querySelector('.comparison-content').scrollTop = 0;
    await this.loadActive();
  }

  async loadActive() {
    const item = this.entry(), key = this.key;
    const epoch = ++this.epoch;
    const graphDuration = this.getProject().graph.duration;
    this.message(tx('Chargement de la référence…')); this.setBusy(true);
    this.renderMetadata(); this.draw();
    try {
      if (!item.initialized) {
        const source = REFERENCES[key];
        let saved;
        try {
          await item.persistence;
          if (!item.ignoreSaved) saved = await savedReference(key);
        } catch { /* Browser storage is optional. */ }
        if (epoch !== this.epoch || !$('comparisonDialog').open) return;
        if (saved?.file) {
          await item.engine.loadFile(saved.file); item.custom = true; item.file = saved.file;
          item.engine.setRegion(saved.start || 0, saved.length || graphDuration);
        } else if (source) {
          await item.engine.loadUrl(source.url, source.name);
          item.custom = false; item.file = null;
          item.engine.setRegion(0, source.length);
        }
        item.initialized = true;
      }
      if (epoch !== this.epoch) return;
      item.engine.setMatchLevels($('matchReferenceLevels').checked);
      this.renderMetadata();
      if (item.engine.hasReference) {
        await this.prepare(false, epoch);
      } else {
        this.message(tx('Importez un enregistrement pour comparer ce graphe.'), false);
        this.draw();
      }
    } catch (error) { if (epoch === this.epoch) this.fail(error); }
    finally { if (epoch === this.epoch) { this.setBusy(false); this.renderMetadata(); } }
  }

  graphChanged(audioChanged = true) {
    const previous = this.key, item = this.entry();
    if (audioChanged) {
      this.listenSerial++;
      item.dirty = true; item.engine.invalidateSynthesis();
      if ($('comparisonDialog').open) {
        this.message(tx('Le graphe a changé. Actualisez la synthèse pour comparer cette version.'));
        this.draw(); this.renderMetadata();
      }
    }
    if ($('comparisonDialog').open && previous !== this.key) this.loadActive();
    this.updatePlayback();
  }

  async prepare(showBusy = true, existingEpoch = null) {
    if (existingEpoch !== null && existingEpoch !== this.epoch) return;
    if (!$('comparisonDialog').open) return;
    const item = this.entry(), epoch = existingEpoch ?? ++this.epoch;
    if (!item.engine.hasReference) { this.message(tx('Importez d’abord un enregistrement.')); return; }
    this.beforePlay();
    this.setBusy(true); this.message(tx('Calcul de la synthèse et des spectres…'));
    try {
      await item.engine.prepareSynthesis(this.getProject().graph, this.audioEngine);
      if (epoch !== this.epoch) return;
      item.dirty = false;
      this.message(tx('Prêt. Cliquez sur A ou B ; le changement garde la même position dans le passage.'));
      this.draw(); this.renderMetadata();
    } catch (error) { if (epoch === this.epoch) this.fail(error); }
    finally { if (epoch === this.epoch && showBusy) this.setBusy(false); }
  }

  async importFile(file) {
    if (!file) return;
    const item = this.entry(), key = this.key, epoch = ++this.epoch;
    const graphDuration = this.getProject().graph.duration;
    this.stop();
    this.setBusy(true); this.message(tx('Décodage du fichier…'));
    try {
      await item.engine.loadFile(file);
      item.custom = true; item.initialized = true; item.file = file; item.ignoreSaved = false;
      item.engine.setRegion(0, graphDuration);
      try { await this.persist(item, key, { file, ...item.engine.region }); }
      catch { this.onError(tx('Le son reste disponible pour cette session, mais le navigateur n’a pas pu le conserver.')); }
      if (epoch !== this.epoch || !$('comparisonDialog').open) return;
      await this.prepare(false, epoch);
    } catch (error) { if (epoch === this.epoch) this.fail(error); }
    finally { if (epoch === this.epoch) { this.setBusy(false); this.renderMetadata(); } $('referenceFile').value = ''; }
  }

  async changeRegion() {
    const item = this.entry(), key = this.key, epoch = this.epoch;
    try {
      this.listenSerial++;
      item.engine.setRegion(Number($('referenceStart').value), Number($('referenceLength').value));
      this.updatePosition(0); this.renderMetadata(); this.draw(); this.updatePlayback();
      if (item.custom && item.file) await this.persist(item, key, { file: item.file, ...item.engine.region });
    } catch (error) { if (epoch === this.epoch && key === this.key) this.fail(error); }
  }

  persist(item, key, value) {
    // Preserve user action order even when IndexedDB opens complete out of order.
    const previous = item.persistence || Promise.resolve();
    item.persistence = previous.catch(() => {}).then(() => savedReference(key, value));
    return item.persistence;
  }

  async resetReference() {
    const item = this.entry(), key = this.key, epoch = ++this.epoch;
    this.stop(); this.setBusy(true); this.message(tx('Restauration de la référence…'));
    item.engine.clearReference(); item.custom = false; item.initialized = false;
    item.file = null; item.ignoreSaved = true;
    this.renderMetadata(); this.draw();
    try {
      try { await this.persist(item, key, null); }
      catch { this.onError(tx('La référence est rétablie pour cette session, mais le navigateur n’a pas pu enregistrer ce choix.')); }
      if (epoch === this.epoch && $('comparisonDialog').open) await this.loadActive();
    } finally { if (epoch === this.epoch) this.setBusy(false); }
  }

  async listen(side) {
    if (this.busy) return;
    const item = this.entry(), key = this.key, serial = ++this.listenSerial;
    try {
      if (item.dirty) await this.prepare();
      if (serial !== this.listenSerial || key !== this.key || !$('comparisonDialog').open || item.dirty || !item.engine.hasReference) return;
      this.beforePlay(); this.side = side;
      if (item.engine.isPlaying) item.engine.switchSide(side);
      else await item.engine.play({ side, loop: $('comparisonLoop').checked, master: this.getProject().master, matchLevels: $('matchReferenceLevels').checked });
      this.updatePlayback();
    } catch (error) { if (serial === this.listenSerial && key === this.key && $('comparisonDialog').open) this.fail(error); }
  }

  stop() {
    this.listenSerial++;
    for (const item of this.entries.values()) item.engine.stop();
    this.updatePosition(0); this.updatePlayback();
  }

  setMaster(value) { for (const item of this.entries.values()) item.engine.setMaster(value); }

  message(text, isError = false) {
    this.messageSource = text;
    this.messageIsError = isError;
    $('comparisonMessage').textContent = translateError(text);
    $('comparisonMessage').classList.toggle('error', isError);
  }

  fail(error) {
    if (error?.name === 'AbortError') return;
    const text = error?.message || String(error);
    this.message(text, true); this.updatePlayback();
  }

  setBusy(value) {
    this.busy = value;
    $('comparisonDialog').setAttribute('aria-busy', String(value));
    for (const id of ['prepareComparison', 'importReference', 'resetReference', 'referenceStart', 'referenceLength']) $(id).disabled = value;
    this.updatePlayback();
  }

  renderMetadata() {
    const item = this.entries.get(this.key), source = REFERENCES[this.key];
    const metadata = item?.engine.referenceMetadata, region = item?.engine.region;
    $('referenceName').textContent = item?.custom ? metadata?.name || item.file?.name || tr('Votre son de référence') : tr(source?.name || metadata?.name || 'Votre son de référence');
    $('referenceMetadata').textContent = metadata ? ui`${fixed(metadata.duration)} s · ${fixed(metadata.sampleRate / 1000, 1)} kHz · ${metadata.channels === 1 ? 'mono' : metadata.channels === 2 ? tx('stéréo') : t('channels', {count:metadata.channels})}${!item.custom && source ? ' · ' + tr(source.excerpt) : tx(' · import local')}` : tx('WAV, MP3, OGG, FLAC ou M4A · jusqu’à 60 Mo / 3 min');
    $('referenceSource').hidden = !source || !!item?.custom;
    if (source) $('referenceSource').href = source.source;
    $('resetReference').hidden = !item?.custom || !source;
    $('comparisonNote').textContent = !item?.custom && source ? tr(source.note) : tx('Choisissez un passage représentatif, puis alternez entre l’enregistrement et le son calculé par votre graphe.');
    $('referenceCredit').innerHTML = !item?.custom && source ? ui`Référence : ${esc(source.author)} · <a href="${source.source}" target="_blank" rel="noopener noreferrer">Pixabay</a> · <a href="https://pixabay.com/service/license-summary/" target="_blank" rel="noopener noreferrer">Licence</a>` : tx('Les fichiers importés restent dans ce navigateur.');
    if (metadata) {
      $('referenceStart').max = Math.max(0, metadata.duration - .25);
      $('referenceStart').value = Number(region.start.toFixed(3));
      $('referenceLength').min = Math.min(.25, metadata.duration);
      $('referenceLength').value = Number(region.length.toFixed(3));
      $('referenceLength').max = Math.min(30, metadata.duration - region.start);
      const duration = Math.min(region.length, this.getProject().graph.duration);
      $('comparisonDuration').textContent = t('duration', {seconds:fixed(duration)});
    } else {
      $('referenceStart').value = 0; $('referenceStart').max = 0;
      $('referenceLength').value = ''; $('referenceLength').min = .25; $('referenceLength').max = 30;
      $('comparisonDuration').textContent = tx('Importez une référence pour choisir le passage à comparer.');
    }
    $('prepareComparison').textContent = item?.dirty ? tx('Actualiser la synthèse') : tx('Recalculer la synthèse');
    this.updatePlayback();
  }

  updatePlayback() {
    if (!$('comparisonDialog')) return;
    const item = this.entries.get(this.key), playing = item?.engine.isPlaying || false;
    for (const id of ['prepareComparison', 'referenceStart', 'referenceLength']) $(id).disabled = this.busy || !item?.engine.hasReference;
    for (const [id, side] of [['listenReference', 'reference'], ['listenSynthesis', 'synthesis']]) {
      $(id).disabled = this.busy || !item?.engine.hasReference;
      $(id).setAttribute('aria-pressed', String(playing && this.side === side));
      $(id).querySelector('i').textContent = playing && this.side === side ? '●' : '▶';
    }
    $('stopComparison').disabled = !playing;
    $('comparisonDialog').dataset.playing = playing ? this.side : 'stopped';
  }

  updatePosition(seconds) {
    this.position = seconds;
    if (!$('comparisonTime')) return;
    const duration = this.entries.get(this.key)?.engine.playbackDuration || 1;
    $('comparisonTime').textContent = ui`${fixed(seconds)} / ${fixed(duration)} s`;
    document.querySelectorAll('.comparison-playhead').forEach(element => { element.style.left = ui`${clamp(seconds / duration, 0, 1) * 100}%`; });
  }

  canvas(id) {
    const canvas = $(id), dpr = Math.min(devicePixelRatio || 1, 2);
    const width = canvas.clientWidth, height = canvas.clientHeight;
    if (!width || !height) return null;
    canvas.width = Math.round(width * dpr); canvas.height = Math.round(height * dpr);
    const context = canvas.getContext('2d'); context.setTransform(dpr, 0, 0, dpr, 0, 0);
    return { context, width, height };
  }

  draw() {
    if (!$('comparisonDialog').open) return;
    const analysis = this.entries.get(this.key)?.engine.getAnalysis();
    const color = { reference: '#d8f78a', synthesis: '#b8a6e8' };
    for (const side of ['reference', 'synthesis']) {
      const result = analysis?.[side], plot = this.canvas(side === 'reference' ? 'referenceWave' : 'synthesisWave');
      if (!plot) continue;
      const { context: ctx, width: w, height: h } = plot;
      ctx.strokeStyle = '#35392f'; ctx.beginPath(); ctx.moveTo(0, h / 2); ctx.lineTo(w, h / 2); ctx.stroke();
      if (result) {
        // A common visual scale retains the peak differences between A and B.
        const gain = side === 'reference' ? analysis.gainReference : analysis.gainSynthesis;
        const maxPeak = Math.max(.01, (analysis.reference?.peak || 0) * analysis.gainReference, (analysis.synthesis?.peak || 0) * analysis.gainSynthesis);
        ctx.strokeStyle = color[side]; ctx.lineWidth = Math.max(1, w / result.waveform.length * .7);
        ctx.beginPath(); result.waveform.forEach(([min, max], index) => { const x = index / result.waveform.length * w; ctx.moveTo(x, h / 2 - max * gain / maxPeak * h * .43); ctx.lineTo(x, h / 2 - min * gain / maxPeak * h * .43); }); ctx.stroke();
      }
      const gainDb = side === 'reference' ? analysis?.gainDbReference : analysis?.gainDbSynthesis;
      $(side === 'reference' ? 'referenceRms' : 'synthesisRms').innerHTML = ui`${result ? fixed((result.matchRmsDb ?? result.rmsDb) + (gainDb || 0), 1) + ' dB' : '—'}<small>RMS 20 Hz–20 kHz</small>`;
    }
    const plot = this.canvas('comparisonSpectrum');
    if (plot) {
      const { context: ctx, width: w, height: h } = plot;
      const left = 36, right = w - 10, top = 10, bottom = h - 23;
      const x = frequency => left + Math.log(frequency / 20) / Math.log(1000) * (right - left);
      const y = db => bottom - clamp((db + 100) / 100, 0, 1) * (bottom - top);
      ctx.font = '10px ui-monospace,monospace'; ctx.fillStyle = '#8c9385'; ctx.strokeStyle = '#30362b'; ctx.lineWidth = 1;
      for (const db of [-20, -40, -60, -80]) { ctx.beginPath(); ctx.moveTo(left, y(db)); ctx.lineTo(right, y(db)); ctx.stroke(); ctx.fillText(String(db), 4, y(db) + 3); }
      for (const hz of [20, 100, 1000, 10000, 20000]) { const px = x(hz); ctx.beginPath(); ctx.moveTo(px, top); ctx.lineTo(px, bottom); ctx.stroke(); ctx.textAlign = hz === 20000 ? 'right' : 'center'; ctx.fillText(hz >= 1000 ? (hz / 1000) + 'k' : String(hz), px, h - 5); }
      ctx.textAlign = 'left';
      for (const side of ['reference', 'synthesis']) {
        const result = analysis?.[side]; if (!result) continue;
        const gainDb = side === 'reference' ? analysis.gainDbReference : analysis.gainDbSynthesis;
        ctx.strokeStyle = color[side]; ctx.lineWidth = 1.8; ctx.beginPath();
        result.spectrum.forEach((point, index) => { const px = x(point.frequency), py = y(point.db + gainDb); index ? ctx.lineTo(px, py) : ctx.moveTo(px, py); }); ctx.stroke();
      }
    }
    $('comparisonBands').innerHTML = [[tx('Grave'), '20–250 Hz', 'lowFraction'], [tx('Médium'), '250 Hz–2 kHz', 'midFraction'], [tx('Aigu'), '2–20 kHz', 'highFraction']].map(([name, range, key]) => ui`<div><span>${name}<small>${range}</small></span><strong class="reference-key">${analysis?.reference ? fixed(100 * analysis.reference[key], 0) + ' %' : '—'}</strong><strong class="synthesis-key">${analysis?.synthesis ? fixed(100 * analysis.synthesis[key], 0) + ' %' : '—'}</strong></div>`).join('');
    this.updatePosition(this.position);
  }

  localeChanged() {
    console.info('[Sona] Refresh comparison language');
    this.renderMetadata();
    if (this.messageSource) this.message(this.messageSource, this.messageIsError);
    this.draw();
    this.updatePosition(this.position);
  }

  bind() {
    console.info('[Sona] Bind comparison controls');
    $('comparisonButton').addEventListener('click', () => this.open());
    $('closeComparison').addEventListener('click', () => $('comparisonDialog').close());
    $('comparisonDialog').addEventListener('close', () => { this.stop(); this.epoch++; this.setBusy(false); });
    $('comparisonDialog').addEventListener('click', event => { if (event.target === $('comparisonDialog')) $('comparisonDialog').close(); });
    $('importReference').addEventListener('click', () => $('referenceFile').click());
    $('referenceFile').addEventListener('change', event => this.importFile(event.target.files[0]));
    $('resetReference').addEventListener('click', () => this.resetReference());
    for (const id of ['referenceStart', 'referenceLength']) $(id).addEventListener('change', () => this.changeRegion());
    $('prepareComparison').addEventListener('click', () => this.prepare());
    $('listenReference').addEventListener('click', () => this.listen('reference'));
    $('listenSynthesis').addEventListener('click', () => this.listen('synthesis'));
    $('stopComparison').addEventListener('click', () => this.stop());
    $('matchReferenceLevels').addEventListener('change', event => { this.entry().engine.setMatchLevels(event.target.checked); this.draw(); });
    $('comparisonLoop').addEventListener('change', () => { if (this.entry().engine.isPlaying) { this.stop(); this.listen(this.side); } });
    $('comparisonDialog').addEventListener('keydown', event => {
      if (event.target.matches('input,select,textarea')) return;
      if (['a', 'b'].includes(event.key.toLowerCase()) && !event.ctrlKey && !event.metaKey) { event.preventDefault(); this.listen(event.key.toLowerCase() === 'a' ? 'reference' : 'synthesis'); }
      if (event.code === 'Space' && !event.target.matches('button')) { event.preventDefault(); this.stop(); }
    });
    new ResizeObserver(() => this.draw()).observe($('comparisonSpectrum'));
  }
}
