// Module Web Worker entry `@etcito/haa/speech-worker` (dist/speech-worker.js, self-contained): renders the speech
// of `speak` nodes off the audio and main threads for the player's `speech` option (speech-host.ts). Start it as a
// module worker: `new Worker(url, { type: 'module' })`.

import { serveSpeech, type SpeechWorkerScope } from './speech-serve.ts';

serveSpeech(self as unknown as SpeechWorkerScope);
