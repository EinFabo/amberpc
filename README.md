# Amber for Windows

Dictation into **any** window - Amber's Android dictation, on the PC. Speak, and the text
appears where your cursor is: in Notepad, the browser, Discord, VS Code, Word.

Local-first: your voice is recognized on this PC by whisper.cpp (CPU or NVIDIA GPU), or -
your choice - by a server with your own API key (Groq, OpenAI, or your own
OpenAI-compatible server).

## Installing

Download `Amber_<version>_x64-setup.exe` from the
[latest release](https://github.com/EinFabo/amberpc/releases/latest) and run it. It installs
for your Windows account only - no administrator rights - into
`%LOCALAPPDATA%\Amber`, with a Start menu entry. Windows SmartScreen may warn on first run
because the installer is not code-signed: *More info → Run anyway*.

On first start a short setup guide picks the engine and downloads what it needs. Amber
then starts with Windows (in the tray, without a window - switch it off under
*Settings → App*) and updates itself from the releases. Uninstall it under *Windows
Settings → Apps*; that also removes the autostart entry, and optionally the downloaded
models.

## Using it

- **Hotkey** (default `Ctrl+Alt+Space`), in three modes: *tap or hold* (a tap starts, the
  next tap stops; holding dictates until you let go), *tap to toggle*, *hold to talk*.
- **The bubble** at the screen edge: click to start and stop, hold to talk, drag to move it
  (it settles at the nearer edge of whichever monitor you drop it on), right-click for the
  settings. It never takes the focus from the window you are writing in.
- **Tray icon**: settings, start/stop, show/hide the bubble, copy the last dictation,
  recognize a failed recording again, updates, quit.
- **Stream Deck** key: hold to talk (see below).

The look is Amber's fire: a flat amber disc with a flame at rest, level bars and sparks
while listening, a rim that fills against the expected wait while recognizing, a tick when
the text is in, red when something failed. The bubble keeps out of full-screen programs
(games, videos) and out of the programs you list; the hotkey works there anyway.

Say "neue Zeile", "neuer Absatz", "Komma", "Fragezeichen", "Punkt" (at a sentence end) - or
"new line", "comma", "question mark" - and the marks go in. A **snippet** phrase on its own
("meine Adresse") inserts its saved text.

## Engines

| Engine | Where your voice goes | Notes |
|---|---|---|
| Local, CPU | nowhere | whisper.cpp `b5454`, 8 MB |
| Local, NVIDIA GPU | nowhere | CUDA 12.4 build with cuBLAS, 653 MB |
| Groq | Groq | Whisper large-v3(-turbo), about $0.04 per hour of speech |
| OpenAI | OpenAI | GPT-4o (mini) Transcribe, Whisper |
| Own server | your server | anything with `POST /v1/audio/transcriptions`; plain `http://` only inside your network |

The local models are the ones Amber for Android uses: **FUTO Base** and **FUTO Small**
(keyboard.futo.org) and **ACFT Large v3 Turbo q8_0** (DeadBranches on Hugging Face, from
MahmoudAshraf's Apache-2.0 fine-tune). All three are tuned so the encoder stops where the
speech stops - Amber sends whisper-server a matching `audio_ctx` per request instead of a
30-second window; on Android that made the large model 4x faster under 8 s of speech, and
more accurate. Amber suggests one by hardware (NVIDIA → Large v3 Turbo, a CPU with room →
Small, an old office PC → Base) and learns how long recognition takes on this PC.

Long dictations are **recognized while you speak**: cut at real pauses, each finished part
recognized while you keep talking, so only the last part is left after you stop.

Runtimes and models are downloaded on demand from those sources, each pinned by URL (Hugging
Face at a fixed commit) and SHA-256 in `crates/core/src/local.rs`; a runtime archive missing
a required DLL is refused. A file with the wrong checksum is deleted, never run. The local
engine runs as `whisper-server.exe` on `127.0.0.1` only, in a Windows job object, so it never
outlives Amber.

With a server engine, a server that cannot be reached (offline, down) hands the recording to
the downloaded local model with a notice - never the other way round.

API keys are encrypted with Windows DPAPI for your account (`secrets.bin`), never written
to the settings file or the log.

## Cleanup

Between recognition and inserting (*Settings → Cleanup*):

- **By rule** - local, instant, nothing invented: spoken punctuation and layout; filler words
  (äh, ähm, öhm, hm, uh, erm …; German words that look like English fillers, "um" and "eh",
  stay); German commas before "dass", "weil", "wenn", "um … zu", "sondern" (Amber for
  Android's rules, 98.5 % right on 4,200 sentences).
- **Smart cleanup** (optional): a language model resolves self-corrections ("um drei, nee
  stopp, um vier" → "um vier"), false starts and punctuation. Locally with llama.cpp
  `b11981` and Qwen3 4B Instruct 2507 (Apache 2.0; CUDA 641 MB + model 2.4 GB, ~3 GB of
  graphics memory), or through Groq, OpenAI or an own server with the Engine page's key.
  The prompt and the guards come from Amber for Android: the dictation is data, never
  instructions; a reply that is cut off, empty, of implausible length or made of words that
  were not dictated (an answer instead of a cleanup) is thrown away and the text goes in as
  recognized. A dictation never waits for a model that is still loading.
- **Per program** (*Settings → Programs*): a writing style - as spoken, casual (chats),
  formal (mail), concise (notes) - an own instruction, and the smart cleanup as set, always
  or never. The program is the window the dictation started in.

## History and statistics

The last dictations - text, program, engine - are kept on this PC, encrypted for the
Windows account, for 7 days by default (*Settings → History*); copy one to put it where it
should have gone. Switching the history off deletes it. A recording whose recognition failed
stays in memory for ten minutes: *Recognize again* (tray or History page) puts the text on
the clipboard. The statistics count words and timings only, never the text: today, the
streak, time saved against typing at 40 words a minute, programs, the wait per engine.

## Stream Deck

`com.einfabo.amber.streamDeckPlugin` (attached to every release; double-click installs it)
adds a *Dictate* key: hold it while you speak, let go to insert - or tap to start and stop.
The key shows the state (flame, listening, working, failed; grey while Amber is not
running). It talks to Amber through a control port on 127.0.0.1 only, with a fresh token
every start, handed over in `%APPDATA%\app.amber.pc\streamdeck.json`.

**Muted in Discord while dictating** (Microphone page; any dictation, not only the Stream
Deck): while Amber listens it presses Discord's own microphone switch - found through UI
Automation, pressed with a click sent to Discord's window as a message, which cannot take
the focus from the game or program in front - and presses it again at the stop. Amber only
takes back its own mute: not one the user set, not while they are deafened; an unmute that
does not take (Discord minimized, closed) is tried again until it does. Nothing to set up
in Discord. Not while Discord is minimized or in the tray, not for dictations into Discord
itself, and only with Discord in English or German (the switch is found by its name).

```bash
cd streamdeck && npm run setup && npm test && npm run pack   # -> dist/
```

## Phone API (optional)

Amber on the Android phone can use this PC as its server - same network, the PC's GPU,
no change to the phone app. Turn it on under *Settings → Phone*; it shows the address to
enter in the phone's *Engine → Own server* (as text and as a QR code), e.g.
`http://192.168.178.30:8765/<key>/v1`.

- OpenAI-compatible: `POST /v1/audio/transcriptions` (multipart, WAV) answers `{"text": …}`,
  `GET /v1/models` lists `amber-pc`. Recognition uses the engine set on the Engine page.
- The phone's own-server engine has no API key field, so the access key is part of the
  path; a `Bearer` header with the key works too. A new key locks out old addresses.
- Only devices in the private network (192.168/16, 10/8, 172.16/12, Tailscale's 100.64/10,
  loopback) are answered. Off by default; Windows asks once to let Amber through its
  firewall.
- Silence answers `{"text": ""}`, which the phone's *Test connection* counts as success.
- The PC's cleanup applies to the phone's dictations too (switch on the Phone page): fillers
  out, and the smart cleanup if it is on - no update of the phone app needed. Leave the
  phone app's own "Chat model" empty, or the text is cleaned twice.

## Inserting

The text goes into the window the dictation started in. If another program is in front by
the time the text is ready, Amber does not type into it: the text goes on the clipboard with
a notice. Amber waits until the hotkey's modifier keys are released, then:

- **Paste** (default): clipboard + `Ctrl+V`, the previous clipboard text restored after
  400 ms. Dictations are kept out of the clipboard history (`Win+V`).
- **Type**: Unicode keystrokes via `SendInput`; the clipboard stays untouched. A line break
  is Shift+Enter, so a chat does not send it.

Windows blocks synthetic input into windows running as administrator. Amber checks for that
up front and leaves the text on the clipboard with a note, instead of pretending it typed.

## Layout

```
crates/core/   pure Rust: audio maths, engines, cleanup, settings, downloads, gestures,
               the hardened HTTP server behind the phone API and the control port  → tests
streamdeck/    the Stream Deck plugin (TypeScript, Elgato SDK v2)
src-tauri/     the Windows app: capture, dictation, insertion, whisper process, bubble
src/           web UI: settings window (React), bubble (canvas)
tools/win.sh   mirrors the tree to C:\Users\fabia\build\amberpc and builds there
```

Same rule as Amber on Android: **whatever can go wrong without a platform API belongs in
`core`**, where it is tested without Windows.

## Building

Release builds started from the build folder (`target\release`) never register themselves for
autostart - only the installed app does.

```bash
cargo test -p amber-core        # anywhere, also in WSL
tools/win.sh install            # npm ci on the Windows side
tools/win.sh check              # cargo check against the real Windows APIs
tools/win.sh test               # all tests on Windows
tools/win.sh build              # signed NSIS installer in target/release/bundle/nsis
tools/win.sh dev                # tauri dev
```

The updater key lives outside the repo in `~/.amberpc/` (`updater.key`,
`updater.password`); `tools/win.sh build` passes it to the build.

Data: `%APPDATA%\app.amber.pc\` (settings, keys), `%LOCALAPPDATA%\app.amber.pc\` (runtimes,
models, `history.bin`, `stats.jsonl`, `estimates.json`, `logs\amber.log`).

## Releases and updates

`tools/release.sh "notes"` runs the tests, builds and signs the installer on this PC and
publishes it as a GitHub release together with `latest.json`. The signing key stays in
`~/.amberpc/` and never goes to GitHub. Bump `version` in `package.json` and `Cargo.toml`
first.

Installed copies check
`github.com/EinFabo/amberpc/releases/latest/download/latest.json` at start and every six
hours, download in the background, and install once no dictation has run for a minute.
GitHub Actions (`.github/workflows/ci.yml`) runs the tests and a Windows build on every push.

## System requirements

Windows 10 1809 or newer, x64. WebView2 (the installer fetches it if missing). For the GPU
engine an NVIDIA card from the GTX 900 series on with a current driver.

## Not in yet

Parakeet (the better choice for PCs without an NVIDIA GPU) and Tinfoil's enclave
verification - both exist in Amber for Android. The roadmap is `docs/PLAN.md`.
