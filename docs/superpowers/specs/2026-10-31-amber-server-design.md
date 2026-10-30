# Amber Server - design

A headless speech-to-text server with the same API as Amber for Windows' phone API, for a
PC, a NAS or a small Linux box that has no desktop: one file, one command to install, CPU or
GPU, Linux or Windows. Amber for Android (and anything else that speaks the OpenAI API) uses
it as its "own server".

## What it does

- Serves `crates/core/src/api.rs` unchanged: `POST /v1/audio/transcriptions`,
  `POST /v1/chat/completions`, `GET /v1/models`, `GET /health`; the access key in the path
  (or as a Bearer token); private peers only; the limits of `crates/core/src/http.rs`.
- Recognizes locally with whisper.cpp built into the binary (`whisper-rs` 0.16, in-process -
  no whisper-server, no second download), or passes audio on to a cloud API (Groq, OpenAI,
  any OpenAI-compatible URL) for machines too small for a model.
- Applies the rule cleanup to transcriptions (`text::by_rule`: fillers, spoken punctuation,
  German commas), each switchable.
- Answers `/v1/chat/completions` through a configured OpenAI-compatible chat endpoint
  (`cleanup::ChatCleaner::forward`) - a cloud provider, or a local Ollama/llama-server the
  user already runs. No bundled LLM in the first version.
- Chooses and downloads a model for the hardware (`local::recommend`, `local::MODELS`,
  `local::download_verified` - pinned URLs and SHA-256).
- Prints the phone's address and a QR code in the terminal.
- Installs itself as a per-user background service (systemd user unit; a Windows scheduled
  task at log-on), so it runs without anyone logged in on Linux (linger) and after log-on
  on Windows.

## What it is not (first version)

No GUI, no Cloudflare tunnel (a later version reuses `src-tauri/src/tunnel.rs`'s approach),
no local LLM, no history or statistics, no macOS build.

## Install

```sh
curl -fsSL https://github.com/EinFabo/amberpc/releases/latest/download/install-server.sh | sh
```
```powershell
irm https://github.com/EinFabo/amberpc/releases/latest/download/install-server.ps1 | iex
```

The script picks the build for the machine, downloads it from the GitHub release, checks it
against `SHA256SUMS` from the same release, puts it in the user's program folder
(`~/.local/bin`, `%LOCALAPPDATA%\Programs\amber-server`), runs `amber-server setup` and
`amber-server service install`, and ends with the address and QR code.

## Builds

| Build | For | whisper.cpp backend |
|---|---|---|
| `cpu` | x86-64 with AVX2 | CPU, AVX2/FMA |
| `cpu-compat` | older x86-64 (Sandy Bridge, AVX only) | CPU, AVX, no AVX2 |
| `vulkan` | any GPU with a Vulkan driver (NVIDIA, AMD, Intel) | Vulkan |

Each for `linux-x64` and `windows-x64`: six files, built by a GitHub Actions workflow on a
`server-v*` tag. Vulkan instead of CUDA: one GPU build for every vendor, and no CUDA
libraries to ship. CUDA can follow if Vulkan is too slow.

## Files and settings

- Config: `amber-server.toml` in `~/.config/amber-server/` or `%APPDATA%\amber-server\`
  (on Linux mode 600: it holds the access key and any cloud key).
- Models: `~/.local/share/amber-server/models/` or `%LOCALAPPDATA%\amber-server\models\`.
- Logs: stdout (journald / the task's log file).

## Commands

`amber-server setup` (hardware check, model choice and download, a new key, the config),
`run` (serve), `url` (addresses and QR code), `key --new`, `service install|uninstall`,
`status`.
