# Amber Server - design

A headless speech-to-text server with the same API as Amber for Windows' phone API
(`docs/phone-api.md`), for a PC, a NAS or a small Linux box without a desktop: one command to
install with sensible defaults, and for those who want it, a command for every setting - models
for recognition and cleanup chosen, downloaded and switched. Linux or Windows, CPU or GPU.
Amber for Android shows the models it offers as a choice (`~/projects/amethyst/docs/AMBER-PC.md`).

## Decisions (Fabi, 2026-10-31)

- **Local only.** No cloud engines in the first version: a cloud key is one more thing a normal
  user has to get right. If it comes later, it comes as a guided step (`amber-server cloud`)
  that tests the key before saving it.
- **Quick and full.** The installer runs a quick setup without questions; `amber-server setup`
  asks, and every setting has its command.
- **Models are downloaded, not bundled**, for recognition and for the cleanup alike.
- The phone chooses among the models the server has downloaded.

## What it does

- Serves `crates/core/src/api.rs` unchanged (`service: "amber-server"`): key in the path, wrong
  keys paused, a request budget, private peers only, the limits of `crates/core/src/http.rs`.
- **Recognition**: whisper.cpp built into the binary (`whisper-rs` 0.16, in-process). Any model
  of `local::MODELS` can be downloaded; one is the default; a request naming another
  downloaded one gets it (the previous one is unloaded - one model in memory).
- **Cleanup**: `llama-server` from llama.cpp's own release builds, downloaded for the platform
  (`linux-x64`: `ubuntu-x64`, `ubuntu-vulkan-x64`; `windows-x64`: `win-cpu-x64`,
  `win-vulkan-x64`; pinned build and SHA-256 like `local::LLM_RUNTIMES`), run as a child
  process bound to 127.0.0.1 with a random `--api-key`, ended with the server. Models from
  `local::LLM_MODELS`; one is the default; a chat request naming another downloaded one
  switches to it (llama-server restarts with it). Off unless a cleanup model is downloaded.
  (Whisper and llama.cpp cannot share one binary: each brings its own ggml.)
- **Rules** on every transcription (`text::by_rule`: fillers, spoken punctuation, German
  commas), each switchable.
- **Model list** (`/v1/models`): every downloaded model of both kinds with label and note
  (size, speed on this machine, CPU or GPU), the defaults marked.
- Phone address and QR code in the terminal; per-user background service (systemd user unit
  with linger; a scheduled task at log-on on Windows).

## Commands

| Command | What it does |
|---|---|
| `amber-server setup --quick` | no questions: hardware check, the recommended speech model, a key, the config (what the installer runs) |
| `amber-server setup` | the same, asking: speech model, cleanup yes/no and which, language, port |
| `amber-server models` | speech models: downloaded (default marked) and available with size and a fit for this machine |
| `amber-server models add <id>` / `remove <id>` / `default <id>` | download, delete, make the default |
| `amber-server cleanup` | the same for cleanup models (`add`, `remove`, `default`, `off`), downloading llama-server with the first one |
| `amber-server config` / `config get <key>` / `config set <key> <value>` | every setting: `port`, `language`, `vocabulary`, `threads`, `rules.fillers`, `rules.spoken_punctuation`, `rules.german_commas`, `gpu` (auto/off) |
| `amber-server run` | serve (what the service runs) |
| `amber-server url` | addresses and QR code |
| `amber-server key --new` | a new access key, and the new address |
| `amber-server service install` / `uninstall` / `status` | the background service |
| `amber-server status` | config, models, service, and the server's own health answer |
| `amber-server logs` | the service's last lines (journalctl / the task's log file) |

## Install

```sh
curl -fsSL https://github.com/EinFabo/amberpc/releases/latest/download/install-server.sh | sh
```
```powershell
irm https://github.com/EinFabo/amberpc/releases/latest/download/install-server.ps1 | iex
```

The script picks the build for the machine, checks it against `SHA256SUMS` from the same
release, installs it (`~/.local/bin`, `%LOCALAPPDATA%\Programs\amber-server`), runs
`setup --quick` and `service install`, and ends with the address and QR code - plus one line
on how to add a cleanup model (`amber-server cleanup add`).

## Builds

| Build | For | whisper.cpp backend | llama-server it downloads |
|---|---|---|---|
| `cpu` | x86-64 with AVX2 | CPU, AVX2/FMA | CPU build |
| `cpu-compat` | older x86-64 (Sandy Bridge, AVX only) | CPU, AVX, no AVX2 | CPU build |
| `vulkan` | any GPU with a Vulkan driver | Vulkan | Vulkan build |

For `linux-x64` and `windows-x64`: six files, built by a GitHub Actions workflow on a
`server-v*` tag. Vulkan instead of CUDA: one GPU build for every vendor and nothing extra to
ship; CUDA can follow.

## Files

- Config `amber-server.toml` in `~/.config/amber-server/` or `%APPDATA%\amber-server\` (mode
  600 on Linux: it holds the access key).
- Models and llama-server in `~/.local/share/amber-server/` or `%LOCALAPPDATA%\amber-server\`.
- Logs to stdout (journald, or the task's log file on Windows).

## Not in the first version

Cloud engines, the Cloudflare tunnel (later: the approach of `src-tauri/src/tunnel.rs`), history
and statistics, macOS, ARM builds.
