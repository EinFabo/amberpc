# Amber for Windows - plan: what to take over from Amber for Android

As of 2026-10-29, after 0.3.1. Source: Amber for Android 0.16 (`~/projects/amethyst`, its
`docs/OPEN.md` and `:core`). Ordered by value for the effort. Every phase ends with tests, a
review by a review agent, the fixes, and a build installed on Fabi's PC.

Legend: `[ ]` open · `[~]` in progress · `[x]` done

Decided by Fabi (2026-10-29): a window change during recognition puts the text on the
clipboard with a notice; history on, kept 7 days, local only; statistics and snippets are in,
Parakeet and translation are not; after each reviewed phase Claude publishes the release.

## Phase 1 - Faster and more accurate local recognition

The biggest lever. Whisper always encodes a full 30 s window; on a GPU that costs
milliseconds, on a CPU it is most of the wait (measured on Fabi's i7-12700K: 3.4 s of speech
took 18.6 s with large-v3-turbo on the CPU). Android 0.16 solved it with ACFT models.

- [x] **ACFT models with a short encoder context.** Fine-tuned so the encoder may stop where
      the speech stops; whisper-server takes `audio_ctx` per request (checked in b5454's
      server.cpp: parameters are copied per request, `/inference` reads `audio_ctx`).
      Measured on Android: 4.1x faster under 8 s of speech, *more* accurate (WER 4.5 % vs
      4.9 %). Models, pinned by URL and SHA-256 as on Android:
  - FUTO Multilingual-74 (Whisper base, 82 MB) - weak PCs
  - FUTO Multilingual-244 (Whisper small, 264 MB) - CPUs
  - ACFT large-v3-turbo q8_0 (874 MB, slack 128) - GPUs and fast CPUs, the new default
  - the stock models stay for existing downloads, marked "replaced"
  - `audio_ctx = ceil(samples / 320) + slack`, at least 256, 0 (= full) from 30 s on - port
    `WhisperInput.audioCtxFor` with its tests
- [x] **Model suggestion by hardware**: NVIDIA → ACFT large on CUDA; else ≥ 8 threads and
      ≥ 8 GB RAM → FUTO 244 on the CPU; else FUTO 74. Shown as "recommended", never
      switched silently.
- [x] **Learned wait estimate** (port `RecognitionEstimate`): per engine and model, the
      recognition time per second of speech on this PC; shown next to each model ("about
      0.4 s for a sentence") and used by the bubble's progress ring (phase 4).
- [x] **Local fallback**: a server that cannot be reached (offline, DNS, timeout - not a
      refused key) → the same recording goes to a downloaded local model, with a notice.
      Never the other way round. Switch, on by default.
- [x] **German comma rules** (port `GermanCommas`, 98.5 % right on 4,200 sentences):
      before "dass", "weil", "wenn", "um ... zu", "sondern". Switch under Cleanup.

## Phase 2 - Text that fits where it goes

- [x] **Profiles per program** (port `AppProfiles`): per exe (discord.exe, outlook.exe,
      code.exe ...) a writing style - as spoken, casual, formal, concise - an own instruction,
      and smart cleanup default/always/never. Defaults: chat apps casual, mail formal, notes
      concise, code editors as spoken. The style goes into the cleanup prompt (port
      `styleRule`); the program is the target window's process. Settings page "Programs" with
      a picker of running programs.
- [x] **Spoken punctuation and layout**, German and English: "neue Zeile", "neuer Absatz",
      "Komma", "Punkt" (only at a sentence end), "Fragezeichen", "Ausrufezeichen",
      "Doppelpunkt", "new line", "comma" ... Rules in `core`, with tests; switch.
- [x] **Own words in the cleanup**: the vocabulary also goes to the smart cleanup as
      "these spellings are correct".
- [x] **When the window changed during recognition**: no longer pull the focus back - the text
      goes on the clipboard with a notice (as on Android). The settings window's test field
      and the bubble (which never takes focus) still count as "the same window".

## Phase 3 - Nothing gets lost

- [x] **History** (on by default): the last dictations kept on this PC - text, program, time, engine -
      with copy and "insert again"; auto-delete after a week; off switch. Tray entry "Insert
      last dictation again".
- [x] **Recognize again**: a recording whose recognition failed (key refused, quota, server
      error) is kept in memory for 10 minutes; one click recognizes it again, also with
      another engine.
- [x] **Statistics**: words and dictations today/this week, per program, median wait
      per engine, time saved compared with typing at 40 wpm - counts and timings only, never
      text (port `StatsLedger`).

## Phase 4 - Bubble and keys

- [x] **Hide the bubble in chosen programs**, and automatically over full-screen apps
      (games, videos); the hotkey and the Stream Deck keep working.
- [x] **Opacity at rest**: the opacity setting applies at rest only; dictating, the bubble
      is fully visible (as on Android).
- [x] **Progress ring**: while recognizing, the fuse fills the rim against the learned
      estimate instead of only circling.
- [x] **Reset the bubble's place** (settings button).

## Phase 5 - Long dictations

- [x] **Recognize while speaking** (port `VadPolicy`, `ChunkPlanner`,
      `ChunkedTranscription`): long dictations are cut at real pauses, finished parts are
      recognized in the background with the text so far as prompt, the language of the first
      part is kept; after the stop only the last part is left. Local engines only (servers are
      fast enough, and some bill per request).

## Phase 6 - Snippets

- [x] **Snippets**: a spoken shortcut ("Snippet Adresse", "meine Signatur") inserts a saved
      text instead of the dictation. Matched after recognition, before the cleanup; a page to
      manage them.

## Phase 7 - Easier servers, models for every PC (Fabi, 2026-11-01)

- [ ] **Amber Server, made user-friendly.** What tripped Fabi up on a real install:
      - `models add futo-multingual-244` (a typo), `add Small`, `add 2` were all refused. The fix:
        take the list's label and number too, and on a miss suggest the closest id ("did you
        mean futo-multilingual-244?").
      - Removing it took the README. The fix: an `amber-server uninstall` command that stops
        the service, removes the program, asks before deleting models and settings, and says
        what is left at Cloudflare (tunnel, DNS record) and whether lingering was switched on
        for it.
      - Further:
        - `amber-server` with no command shows the status and the next step, not the help.
        - `setup` as one guided run: model by hardware, key, service, tunnel optional.
        - `amber-server update`.
        - Error messages that end with the command that fixes them.
        - Maybe a small status page on localhost (model, keys, requests today, the log),
          behind the key.
      - The PC's Phone page gets the same review: what a newcomer must understand before the
        phone connects.
- [ ] **Cleanup models besides Qwen3 4B, also for CPU-only PCs.** Qwen3 4B (2.4 GB) is slow
      without a GPU.
      - Candidates, all GGUF for llama.cpp:
        - Qwen3 1.7B and Qwen3.5 2B, the same family, good at German.
        - Gemma 4 E2B (~2 GB, quantization-aware GGUF from Google).
        - Llama 3.2 3B (German officially supported).
        - Phi-4-mini (3.8B).
        - Too small for real cleanup: Qwen3 0.6B and Gemma 3 1B, as a last resort for weak
          PCs.
      - How to choose: measure each on Fabi's PC, CPU only and on CUDA, with the cleanup's own
        prompt. Run the dictations that already showed problems: self-corrections ("um drei
        nee um vier"), lists, short German that must not be translated, a question that must
        not be answered. Measure speed (ms per dictation, RAM) and how often `plausible`
        rejects a reply.
      - Offer the best one or two next to Qwen3 4B, and suggest one by hardware as the speech
        model is.
      - Same catalog for Amber Server.
- [ ] **A better speech model for PCs without an NVIDIA GPU.** FUTO Base/Small are fast but
      weak; Large v3 Turbo is too slow on a CPU.
      - Candidates:
        - **Parakeet TDT 0.6B v3** (NVIDIA, CC-BY-4.0, 25 European languages incl. German,
          near Whisper large-v3 accuracy, many times faster on a CPU). Amber for Android runs
          it through sherpa-onnx since 0.10.
        - Nemotron 3.5 ASR (NVIDIA, June 2026, 600M, streaming, 40 locales).
        - Qwen3-ASR (Alibaba, January 2026; accurate, but heavier on a CPU).
        - Whisper on AMD/Intel graphics through whisper.cpp's Vulkan build (the server
          workflow already builds Vulkan).
      - How to choose: measure word error rate on real German speech (MINDS-14 / FLEURS as in
        the turbo check) and the wait on Fabi's CPU with the GPU unused, against FUTO Small and
        Large v3 Turbo.
      - Parakeet needs a second runtime (sherpa-onnx or ONNX Runtime) next to whisper.cpp. The
        language list, the vocabulary prompt and the chunking must work with it, and it does
        not detect languages the way Whisper does.

## Not planned

- Translation while dictating (Fabi, 2026-10-29: not now). Parakeet moved to phase 7.
- Tinfoil's enclave verification (Go verifier library, large); Android-only parts (keyboard
  tracking, accessibility, password-field detection through Android APIs).
