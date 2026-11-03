import { PCM_SAMPLE_RATE, pcm16ToFloat32 } from './pcm';
import { loadSpeechEncoder, type SpeechEncoder } from './speech-encoding';

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const text = $<HTMLTextAreaElement>('text');
const voice = $<HTMLSelectElement>('voice');
const speed = $<HTMLInputElement>('speed');
const speedLabel = $<HTMLSpanElement>('speed-label');
const speak = $<HTMLButtonElement>('speak');
const stop = $<HTMLButtonElement>('stop');
const status = $<HTMLParagraphElement>('status');
const audio = $<HTMLAudioElement>('audio');

interface ManagedMediaSourceGlobal {
  ManagedMediaSource?: typeof MediaSource;
}
const MediaSourceClass =
  (window as ManagedMediaSourceGlobal).ManagedMediaSource ??
  (typeof MediaSource === 'undefined' ? null : MediaSource);

let encoder: Promise<SpeechEncoder | null> | null = null;
let current: AbortController | null = null;

/** Loaded at startup, so the first click's play() still runs within its user activation. */
function getEncoder(): Promise<SpeechEncoder | null> {
  if (!MediaSourceClass) return Promise.resolve(null);
  encoder ??= loadSpeechEncoder(MediaSourceClass, PCM_SAMPLE_RATE).catch((err: unknown) => {
    encoder = null; // Let the next click try again.
    throw err;
  });
  return encoder;
}

speed.addEventListener('input', () => (speedLabel.textContent = `${Number(speed.value).toFixed(2)}×`));

async function loadVoices() {
  const response = await fetch('/v1/audio/voices');
  if (!response.ok) throw new Error(`voices: HTTP ${response.status}`);
  const { voices } = (await response.json()) as { voices: string[] };
  voice.replaceChildren(...voices.map((name) => new Option(name, name, false, name === 'af_heart')));
}

function setStatus(message: string) {
  status.textContent = message;
}

async function* body(response: Response): AsyncGenerator<Uint8Array> {
  const reader = response.body!.getReader();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) return;
      yield value;
    }
  } finally {
    // Also stops the server's synthesis when we stop reading early.
    await reader.cancel().catch(() => {});
  }
}

function append(buffer: SourceBuffer, bytes: Uint8Array): Promise<void> {
  return new Promise((resolve, reject) => {
    buffer.addEventListener('updateend', () => resolve(), { once: true });
    buffer.addEventListener('error', () => reject(new Error('SourceBuffer error')), { once: true });
    buffer.appendBuffer(bytes as Uint8Array<ArrayBuffer>);
  });
}

function stopPlayback() {
  current?.abort();
  current = null;
  audio.pause();
  audio.removeAttribute('src');
  audio.load();
}

function untilOpen(source: MediaSource, signal: AbortSignal): Promise<void> {
  if (source.readyState === 'open') return Promise.resolve();
  return new Promise((resolve, reject) => {
    source.addEventListener('sourceopen', () => resolve(), { once: true, signal });
    signal.addEventListener('abort', () => reject(signal.reason), { once: true });
  });
}

async function play() {
  stopPlayback();
  if (!MediaSourceClass) return setStatus('This browser has no Media Source Extensions.');
  const request = new AbortController();
  current = request;
  const isCurrent = () => current === request;
  const started = performance.now();
  setStatus('Requesting…');

  const source = new MediaSourceClass();
  audio.disableRemotePlayback = true;
  const url = URL.createObjectURL(source);
  audio.src = url;
  // Called before any await, so it's within the click's user activation.
  const playing = audio.play().catch(() => {});
  audio.addEventListener('error', () => isCurrent() && setStatus('Error: the audio element failed to decode.'), {
    once: true,
    signal: request.signal,
  });

  try {
    const speechEncoder = await getEncoder();
    if (request.signal.aborted) return;
    if (!speechEncoder) return setStatus("This browser can't encode audio MSE will play.");
    const response = await fetch('/v1/audio/speech', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        model: 'kokoro',
        input: text.value,
        voice: voice.value,
        speed: Number(speed.value),
        response_format: 'pcm',
        stream: true,
      }),
      signal: request.signal,
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}: ${(await response.text()).slice(0, 300)}`);
    await untilOpen(source, request.signal);
    const buffer = source.addSourceBuffer(speechEncoder.mimeType);
    buffer.mode = 'sequence';
    let firstPcm: number | null = null;
    let samples = 0;
    async function* timed(pieces: AsyncIterable<Float32Array>) {
      for await (const piece of pieces) {
        firstPcm ??= performance.now() - started;
        samples += piece.length;
        yield piece;
      }
    }
    audio.addEventListener(
      'playing',
      () =>
        setStatus(
          `Playing. First PCM after ${firstPcm?.toFixed(0)} ms, sound after ${(performance.now() - started).toFixed(0)} ms (${speechEncoder.mimeType}).`
        ),
      { once: true, signal: request.signal }
    );
    for await (const fragment of speechEncoder.encode(timed(pcm16ToFloat32(body(response))), PCM_SAMPLE_RATE)) {
      if (request.signal.aborted) return;
      await append(buffer, fragment);
    }
    if (source.readyState === 'open') source.endOfStream();
    await playing;
    if (!isCurrent()) return;
    if (samples === 0) setStatus('Nothing to say (the server skipped all of it).');
    else setStatus(`${status.textContent} Received ${(samples / PCM_SAMPLE_RATE).toFixed(1)} s of audio in ${((performance.now() - started) / 1000).toFixed(2)} s.`);
  } catch (err) {
    if (request.signal.aborted) return;
    setStatus(`Error: ${err instanceof Error ? err.message : String(err)}`);
    if (source.readyState === 'open') source.endOfStream('network');
  } finally {
    // Whatever ended this request, stop its fetch so the server stops synthesizing.
    request.abort();
    URL.revokeObjectURL(url);
  }
}

speak.addEventListener('click', () => void play());
stop.addEventListener('click', () => {
  stopPlayback();
  setStatus('Stopped.');
});
loadVoices().catch((err) => setStatus(`Couldn't load voices: ${err}`));
getEncoder().catch((err) => setStatus(`Couldn't load the encoder: ${err}`));
