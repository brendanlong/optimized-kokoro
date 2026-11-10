# optimized-kokoro

A text-to-speech server for the [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) model, built to be safe to put on the internet, fast, and easy to host. It speaks a small subset of the OpenAI-style speech API that [Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI) serves: streamed raw PCM, one voice per request.

It's the TTS behind [Lion Reader](https://lionreader.com/) ([source](https://github.com/brendanlong/lion-reader)), and a project by [Brendan Long](https://www.brendanlong.com/pages/about-me.html).

## Goals

- **Run Kokoro safely.** Kokoro's usual stack parses untrusted text in Python and C. Its text-to-phoneme step falls back to [espeak-ng](https://github.com/espeak-ng/espeak-ng), a C library in which fuzzing has found buffer overflows ([CVE-2023-49990](https://nvd.nist.gov/vuln/detail/CVE-2023-49990), [-49991](https://nvd.nist.gov/vuln/detail/CVE-2023-49991), [-49992](https://nvd.nist.gov/vuln/detail/CVE-2023-49992), [-49993](https://nvd.nist.gov/vuln/detail/CVE-2023-49993)). The latest release, 1.52.0, is from December 2024, and later memory-safety fixes such as [espeak-ng#2428](https://github.com/espeak-ng/espeak-ng/pull/2428) are only on its master branch. Testing our port against 1.52.0 under AddressSanitizer and Valgrind still found out-of-bounds and uninitialised reads on generated input (including in `CountVowelPosition`, the function behind CVE-2023-49991). Here, everything from the network is handled in Rust with no `unsafe` code of our own, apart from one audited read-only memory map of the Japanese dictionary file (which request text can't affect). The neural network runs in a separate gVisor sandbox that only ever receives small, validated arrays of numbers.
- **Improve performance.** No Python on the request path: text processing (normalization, sentence splitting and phonemes) is about 7× faster than the Python pipeline it replaces (6.6–7.2× in our measurements). The model runs compiled (`torch.compile`, fp16).
- **Best-effort multilingual support.** All 9 of Kokoro's languages: American and British English, Spanish, French, Hindi, Italian, Brazilian Portuguese, Mandarin Chinese and Japanese. Text in other scripts inside a sentence (a Russian or Georgian word in English, say) is read in its own language where espeak-ng can, and as "Thai text" and the like where nothing can, rather than letter by letter.
- **Simplify.** Kokoro's upstream stack has many separate paths from text to phonemes. Here there's one pipeline per language, and features no client uses (SSML, voice blending, inline voice changes, formats other than PCM) are gone.
- **Fix edge cases.** Crashes in the original pipeline (some Japanese numbers, odd Unicode) become sensible readings, and symbol and number handling is being improved (issue #15).
- **Be easy to host despite the paranoia.** One install script sets up three containers on Docker: the GPU worker, the server and a demo page. Each runs in gVisor as `nobody`, with a read-only filesystem and no capabilities. By default it listens on localhost only, and an optional Cloudflare Tunnel publishes it without opening any port to the network.

## What it's based on

- [hexgrad/kokoro](https://github.com/hexgrad/kokoro) (the model code) and the [Kokoro-82M weights](https://huggingface.co/hexgrad/Kokoro-82M), through [Brendan's fork](https://github.com/brendanlong/kokoro) with inference cleanups and mixed-script support.
- [hexgrad/misaki](https://github.com/hexgrad/misaki), Kokoro's text-to-phoneme library, ported to Rust: English (with its dictionaries), Chinese (with jieba's and pypinyin's data) and Japanese (cutlet, on MeCab's algorithm with UniDic).
- [espeak-ng](https://github.com/espeak-ng/espeak-ng) 1.52, whose text-to-phoneme engine is ported to Rust function by function and uses its original rule and dictionary files.
- [spaCy](https://spacy.io/)'s English tokenizer and `en_core_web_sm` tagger, ported to Rust.
- [Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI) (through [Brendan's fork](https://github.com/brendanlong/Kokoro-FastAPI)), whose API, text normalization, sentence chunking and audio post-processing this replaces.
- Ideas from [Kokoros](https://github.com/lucasjinreal/Kokoros) and [Crane](https://github.com/lucasjinreal/Crane), Rust Kokoro implementations.

Each port was checked against the original on large corpora (English books, and Wikipedia plus generated and fuzzed text in each language) and matches its output exactly, except where we deliberately differ or the original crashes. Every difference is listed with its reason in [`docs/divergences.md`](docs/divergences.md). The design and its history are in [`docs/plan.md`](docs/plan.md).

## How it works

```
client ──HTTPS──▶ server (Rust) ──Unix socket──▶ worker (PyTorch, GPU)
                  text → phonemes                phoneme ids → audio
```

- **The server** handles HTTP, authentication and limits, cleans up the text, splits it into sentences, turns them into phonemes, and streams 24 kHz 16-bit PCM back as each sentence is rendered. It queues the sentence a listener is waiting for ahead of ones being fetched early.
- **The worker** runs Kokoro on the GPU. It speaks a small binary protocol ([`docs/worker-protocol.md`](docs/worker-protocol.md)), so it never sees text or HTTP, and it re-checks every message it receives.

## Using it

A subset of the OpenAI-style API Kokoro-FastAPI clients use, which is what Lion Reader sends. Other fields, formats and non-streaming requests get a 400:

```sh
curl -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"model": "kokoro", "voice": "af_heart", "input": "Hello there.", "response_format": "pcm", "stream": true}' \
  https://your-host/v1/audio/speech > out.pcm    # 24 kHz mono, 16-bit little-endian
curl -H "Authorization: Bearer $KEY" https://your-host/v1/audio/voices
```

`speed` (0.25 to 4) is optional. The text may include `[pause:1.5s]` and pronunciation overrides like `[Kokoro](/kˈOkəɹO/)`. Only raw PCM is returned; clients encode it themselves (Lion Reader and the demo page use AAC or Opus in fragmented MP4).

`priority` (an integer 0 to 255, higher more urgent, default 128) is optional too, up to the API key's maximum (a higher one runs at the maximum). Every response once the priority is known, including a `503` for load, says which priority the request got in an `X-Priority` header. More urgent requests are always rendered first. When the server estimates a request would wait too long for its first audio (800 ms by default), it refuses it at once with `503` and `Retry-After: 1`, the least urgent and newest first, before any audio: send that request to another provider rather than retrying it here. A `429` means the key already has as many requests in progress as it may. A response that has started is never cut short by load.

## Hosting it

You need Linux with an NVIDIA GPU, Docker with [gVisor](https://gvisor.dev/) (`runsc`), and the NVIDIA Container Toolkit with CDI. The server binary needs an x86-64 CPU from about 2013 or later (Intel Haswell or AMD Zen). Installing, updating, API keys, the Cloudflare Tunnel and a post-install checklist are in [`deploy/README.md`](deploy/README.md):

```sh
git clone https://github.com/brendanlong/optimized-kokoro && cd optimized-kokoro
sudo deploy/install.sh
```

## License

The combined server is GPL-3.0-or-later, because it includes the port of espeak-ng. The crates that don't depend on espeak-ng (text normalization, the English, Chinese and Japanese text-to-phoneme ports, the spaCy port) are Apache-2.0, as is the rest of the repository; see each crate's `Cargo.toml` and license files. Kokoro's weights and voices are Apache-2.0. Bundled data keeps its own licenses: the dictionaries and model data in the Apache crates are MIT, BSD or Unicode-licensed (each crate's `PROVENANCE.md`), and espeak-ng's data is GPL (`crates/espeak-ng/LICENSE` and `NOTICE`).
