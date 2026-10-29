# Amber for Windows (amberPC) - design

Date: 2026-10-29 · Status: approved in chat ("build it without pausing, review along the way")

## Goal

Amber's dictation on the PC: one installable Windows exe that turns speech into text and
pastes it into whatever window has focus. Locally on the PC's own hardware or through a
server with an API key. Installs per user, updates itself.

Done means: install the setup exe, pick a microphone, press a hotkey (or click the bubble),
speak, and the text lands at the cursor in Notepad, Chrome, Discord, VS Code, Word.

## Scope of version 0.1 (option A)

In:
- Floating bubble at the screen edge: click toggles, press-and-hold dictates while held,
  drag moves it (it snaps to the nearest left/right edge and remembers its place).
- Global hotkey with three modes: toggle, hold (push to talk), or both (a tap toggles, a
  hold talks). A second, optional hotkey shows/hides the bubble.
- Microphone choice, with a live level meter in the settings.
- Engines: **Local** (whisper.cpp, downloaded on demand: CPU or CUDA runtime, a choice of
  models) and **Server** (Groq, OpenAI, own OpenAI-compatible server) with an API key.
- Insert into the focused window: paste (clipboard + Ctrl+V, old clipboard text restored)
  or type (Unicode keystrokes, clipboard untouched).
- Settings window for all of the above, plus language(s), bubble look, autostart, updates.
- Tray icon, single instance, NSIS installer (per user, no admin), signed auto-updates
  from GitHub releases.

Later, not in 0.1: cleanup / writing style per app, statistics, history, Parakeet,
Tinfoil enclave, transcription while speaking (chunking).

## System requirements (measured on Fabi's PC)

i7-12700K (12C/20T), 32 GB RAM, RTX 3070 8 GB (driver 617), Windows 11 Home 26200,
WebView2 156. Rust 1.95, Node 24, VS 2022 (MSVC) on Windows.

- Minimum for users: Windows 10 1809+ x64 with WebView2 (the installer fetches it if
  missing). Server engine: any PC. Local CPU: 4+ cores, `base`/`small` models.
- Local GPU: NVIDIA with a driver for CUDA 11.8+ (GTX 900 and newer). The CUDA runtime is
  a 271 MB download, the turbo model 547 MB.
- On this PC: large-v3-turbo on the RTX 3070 is the default recommendation.

## Architecture

```
amberPC/
  crates/core/   amber-core: pure Rust, no Tauri, no Windows API -> unit-tested in WSL
  src-tauri/     the Windows app: capture, hotkeys, bubble, insertion, whisper process
  src/           web UI (React + TS): settings.html, bubble.html
  tools/win.sh   syncs the tree to C:\Users\fabia\build\amberpc and runs npm/cargo there
```

Same ground rule as Amber: **everything that can go wrong without a platform API belongs
in `core`**, where it is tested. `src-tauri` only holds what needs Windows or Tauri.

### core (amber-core)

| Module | Does |
|---|---|
| `audio` | mono downmix, resampling to 16 kHz, WAV encoding, level (RMS -> 0..1), "is there speech at all" |
| `settings` | the settings schema with defaults, validation, atomic load/save as JSON |
| `providers` | Groq / OpenAI / own server: base URLs, speech models; private-host check for http:// |
| `engine` | `Transcriber` trait; `OpenAiTranscriber` (`/audio/transcriptions`) and `WhisperServerTranscriber` (whisper.cpp `/inference`); errors as user-readable messages |
| `text` | transcript cleanup: sound annotations, Whisper's subtitle hallucinations, whitespace; final text with optional trailing space |
| `local` | catalogs of whisper.cpp runtimes and models (pinned URLs + SHA-256), download with progress, verification, atomic install, zip extraction |
| `gesture` | the hotkey/mouse gesture machine: press/release with timestamps -> start / stop / nothing, per mode |

### src-tauri (Windows)

| Module | Does |
|---|---|
| `capture` | cpal input on the chosen device (falls back to the default and says so); owns the stream on its own thread; level events ~30/s; hard cap 10 min |
| `dictation` | the state machine Idle -> Recording -> Transcribing -> Inserting -> Idle/Error; one dictation at a time; emits state to the bubble |
| `insert` | remembers the foreground window when recording starts, gives focus back before inserting, waits for modifiers to be released, then paste or type via `SendInput` |
| `whisper` | runs `whisper-server.exe` on 127.0.0.1 with a free port, no console window, in a job object so it dies with the app; restarts it when model/runtime change |
| `secrets` | API keys encrypted with Windows DPAPI (per user) in the app data folder, never in settings.json |
| `bubble` | the bubble window: transparent, always on top, no taskbar entry, never takes focus (`WS_EX_NOACTIVATE`) |
| `tray`, `hotkeys`, `updates` | tray menu; global shortcuts (pressed/released) into the gesture machine; updater checks at start and every 6 h |

### Data flow

```
hotkey / bubble press ─► gesture ─► dictation.start ─► capture (level ─► bubble)
hotkey / bubble release ─► gesture ─► dictation.stop ─► samples
  ─► audio: mono, 16 kHz, speech check ─► WAV
  ─► engine (local whisper-server | OpenAI-compatible server)
  ─► text cleanup ─► insert into the remembered window ─► bubble: done / error
```

### Bubble

A 64 px amber disc in Amber's style: a flame glyph at rest, level bars while listening, a
spark running around the rim while transcribing, a tick on success, red on failure.
Options: always visible / only while dictating / hidden; edge left or right; size; opacity.
The window never takes focus, so clicking it leaves the cursor where it was. As a second
line of defense the foreground window is remembered at the start and restored before
inserting.

### Errors

Every failure ends in Idle with the bubble red for 2 s and a Windows notification in plain
words: no microphone, nothing heard, key rejected (401), rate limit (429), server
unreachable, local model missing, runtime failed to start. Details go to the diagnostics
log (last 200 events, shown in the settings, no transcript text unless enabled).

### Storage

`%APPDATA%\app.amber.pc\settings.json` (settings), `secrets.bin` (DPAPI),
`%LOCALAPPDATA%\app.amber.pc\runtime\{cpu,cuda}\` and `models\` (downloads).

### Updates and releases

tauri-plugin-updater against `github.com/EinFabo/amberpc/releases/latest/download/latest.json`,
minisign key outside the repo (`~/.amberpc/updater.key`), GitHub Actions release on a `v*`
tag like ClippiBoy. NSIS `currentUser`, update `installMode: passive`.

## Testing

- `cargo test -p amber-core` in WSL: audio maths, WAV header, settings round trip and
  validation, provider/host rules, response parsing and error mapping against a local test
  HTTP server, text cleanup, catalog integrity, download verification, gesture machine.
- Windows build: `tools/win.sh build` -> `cargo test` + `tauri build`; manual check list:
  dictate into Notepad (paste and type), Chrome address bar, a hold on the hotkey, a click
  on the bubble without losing focus, local CPU + CUDA, Groq with a key.
- Reviews by a reviewer agent after the core, after the app, and before the release.

## Plan (milestones)

1. Workspace + core modules with tests.
2. Tauri app skeleton: tray, settings window, bubble window, single instance, build on Windows.
3. Capture + dictation + insertion + server engine -> first real dictation.
4. Local engine: runtime/model download, whisper-server process.
5. Settings UI complete, hotkey recorder, mic meter, diagnostics.
6. Installer, icons, updater, release workflow. Review, fixes, 0.1.0.
