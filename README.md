# Amber for Windows

Dictation into **any** window - Amber's Android dictation, on the PC. Speak, and the text
appears where your cursor is: in Notepad, the browser, Discord, VS Code, Word.

Local-first: your voice is recognized on this PC by whisper.cpp (CPU or NVIDIA GPU), or -
your choice - by a server with your own API key (Groq, OpenAI, or your own
OpenAI-compatible server).

## Using it

- **Hotkey** (default `Ctrl+Alt+Space`), in three modes: *tap or hold* (a tap starts, the
  next tap stops; holding dictates until you let go), *tap to toggle*, *hold to talk*.
- **The bubble** at the screen edge: click to start and stop, hold to talk, drag to move it
  (it settles at the nearer edge of whichever monitor you drop it on), right-click for the
  settings. It never takes the focus from the window you are writing in.
- **Tray icon**: settings, start/stop, show/hide the bubble, updates, quit.

The look is Amber's fire: a flat amber disc with a flame at rest, level bars and sparks
while listening, a fuse running around the rim while recognizing, a tick when the text is
in, red when something failed.

## Engines

| Engine | Where your voice goes | Notes |
|---|---|---|
| Local, CPU | nowhere | whisper.cpp `b5454`, 8 MB; Base or Small models are comfortable |
| Local, NVIDIA GPU | nowhere | CUDA 11.8 build, 272 MB; Large v3 Turbo recommended |
| Groq | Groq | Whisper large-v3(-turbo), about $0.04 per hour of speech |
| OpenAI | OpenAI | GPT-4o (mini) Transcribe, Whisper |
| Own server | your server | anything with `POST /v1/audio/transcriptions`; plain `http://` only inside your network |

Runtimes and models are downloaded on demand from their official sources (GitHub releases
of ggml-org/whisper.cpp, ggerganov/whisper.cpp on Hugging Face), each pinned by URL and
SHA-256 in `crates/core/src/local.rs`. A file with the wrong checksum is deleted, never
run. The local engine runs as `whisper-server.exe` on `127.0.0.1` only, in a Windows job
object, so it never outlives Amber.

API keys are encrypted with Windows DPAPI for your account (`secrets.bin`), never written
to the settings file or the log.

## Inserting

The window that had focus when the dictation started gets focus back, Amber waits until the
hotkey's modifier keys are released, then:

- **Paste** (default): clipboard + `Ctrl+V`, the previous clipboard text restored after
  400 ms. Dictations are kept out of the clipboard history (`Win+V`).
- **Type**: Unicode keystrokes via `SendInput`; the clipboard stays untouched.

Windows blocks synthetic input into windows running as administrator. Amber checks for that
up front and leaves the text on the clipboard with a note, instead of pretending it typed.

## Layout

```
crates/core/   pure Rust: audio maths, engines (HTTP), settings, downloads, gestures  → tests
src-tauri/     the Windows app: capture, dictation, insertion, whisper process, bubble
src/           web UI: settings window (React), bubble (canvas)
tools/win.sh   mirrors the tree to C:\Users\fabia\build\amberpc and builds there
```

Same rule as Amber on Android: **whatever can go wrong without a platform API belongs in
`core`**, where it is tested without Windows.

## Building

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
models, `logs\amber.log`).

## Releases and updates

A `v*` tag builds the installer in GitHub Actions and publishes it with `latest.json`
(`.github/workflows/release.yml`). Installed copies check
`github.com/EinFabo/amberpc/releases/latest/download/latest.json` at start and every six
hours, download in the background, and install once no dictation has run for a minute.

## System requirements

Windows 10 1809 or newer, x64. WebView2 (the installer fetches it if missing). For the GPU
engine an NVIDIA card from the GTX 900 series on with a current driver.

## Not in yet

Cleanup and writing style per app, statistics, history, Parakeet, Tinfoil enclave,
recognition while speaking - all exist in Amber for Android.
