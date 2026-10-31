# Amber Server

Amber's speech-to-text for the phone, on a machine without a desktop: a PC, a NAS, a small
Linux box. It speaks the same API as Amber for Windows' phone API (`docs/phone-api.md`):
recognition with whisper.cpp, and optionally a cleanup with a local language model
(llama.cpp). Everything runs on the machine; nothing goes to a cloud.

## Install

Linux (x86-64):

```sh
curl -fsSL https://github.com/EinFabo/amberpc/releases/download/server-latest/install-server.sh | sh
```

Windows (PowerShell, no admin rights needed):

```powershell
irm https://github.com/EinFabo/amberpc/releases/download/server-latest/install-server.ps1 | iex
```

The installer:

1. picks the build for the machine: `vulkan` for a GPU from NVIDIA or AMD with a Vulkan
   driver, `cpu` for a processor with AVX2, `cpu-compat` for older ones (it takes the CPU
   build when the GPU build does not start; `AMBER_SERVER_FORCE_BUILD=cpu` picks one yourself),
2. checks it against the release's `SHA256SUMS`,
3. installs it: `~/.local/bin/amber-server`, or `%LOCALAPPDATA%\Programs\amber-server` (put on
   your PATH),
4. runs `amber-server setup --quick`: the recommended speech model for this machine, an
   access key,
5. runs `amber-server service install`: started with the machine (Linux) or at log-on (Windows),
6. shows the phone's address with a QR code.

Run the same command again to update. Settings and models stay.

On Linux the binary needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later).

## On the phone

In Amber: Engine -> Own server, and the address `amber-server url` shows (scan the QR code).
The phone and the server must be in the same network. Amber versions that know Amber's model
list offer the server's downloaded models to choose from, for recognition and for the cleanup;
a request for a model the server does not have gets the server's default.

## Commands

| Command | What it does |
|---|---|
| `amber-server setup` | set up, asking: speech model, cleanup, language, port (Enter keeps what is there) |
| `amber-server setup --quick` | the same without questions (what the installer runs) |
| `amber-server models` | the speech models: downloaded (`*` the default) and available, with how they suit this machine |
| `amber-server models add futo-multilingual-74` | download one |
| `amber-server models default futo-multilingual-74` | the one a request without a known model gets |
| `amber-server models remove acft-large-v3-turbo-q8_0` | delete one (not the default) |
| `amber-server cleanup` | the cleanup models, and whether the cleanup is on |
| `amber-server cleanup add qwen3-4b-instruct-2507-q4km` | download one (and llama.cpp with the first) and switch the cleanup on |
| `amber-server cleanup default <id>` / `remove <id>` | as for the speech models |
| `amber-server cleanup off` | switch the cleanup off (the files stay) |
| `amber-server config` | every setting |
| `amber-server config get language` / `config set language de` | one setting |
| `amber-server url` | the phone's address and QR code |
| `amber-server key --new` | a new access key: every phone with the old address is locked out |
| `amber-server status` | settings, models, the service, and whether the server answers |
| `amber-server logs` | the service's last lines |
| `amber-server service install` / `uninstall` / `status` | the background service |
| `amber-server run` | serve in the foreground (what the service runs) |

A running service takes changed settings and models when it restarts: `amber-server service
install` restarts it (as does `key --new` by itself).

## Settings

`~/.config/amber-server/amber-server.toml` (Linux, readable only by you: it holds the key) or
`%APPDATA%\amber-server\amber-server.toml`. Set them with `amber-server config set`:

| Key | Default | Meaning |
|---|---|---|
| `port` | `8765` | the port the phone connects to (1024 and up) |
| `language` | `auto` | the language when the phone sends none (`de`, `en`, ...) - also for the rules |
| `vocabulary` | empty | words to recognise, comma-separated (names, terms); added to the phone's own |
| `threads` | `0` | CPU threads for recognition and cleanup; 0 picks a good number |
| `gpu` | `auto` | `off` runs the GPU build on the CPU |
| `rules.fillers` | `true` | drop "ähm", "uh" and the like |
| `rules.spoken_punctuation` | `true` | "Komma", "new line" ... become punctuation |
| `rules.german_commas` | `true` | commas before German subordinate clauses |

`key`, `speech` and `cleanup` are set by `setup`, `key --new`, `models default` and `cleanup`.
Models and llama.cpp live in `~/.local/share/amber-server/` or `%LOCALAPPDATA%\amber-server\`.

## The cleanup

Recognition alone gives the words; the cleanup (a local language model) tidies punctuation and
slips, and is what Amber on the phone asks for after a dictation. It is a large download (about
2.4 GB) and wants about 4 GB of free memory - on a CPU it takes a few seconds per dictation,
with a GPU well under one.

```sh
amber-server cleanup add qwen3-4b-instruct-2507-q4km
amber-server service install     # restart with it
```

The server loads the model when it starts; the phone gives up on a cleanup after 12 seconds,
so the first one right after a start may come back untidied.

## Firewall

The phone reaches the server on its port (8765):

- Ubuntu with ufw: `sudo ufw allow 8765/tcp`; firewalld: `sudo firewall-cmd --add-port=8765/tcp --permanent && sudo firewall-cmd --reload`.
- Windows asks the first time the server starts: allow private networks.

The server answers only devices in private networks (home network, Tailscale) and only with the
key in the address; wrong keys are paused.

## Uninstall

```sh
amber-server service uninstall
rm ~/.local/bin/amber-server
rm -r ~/.config/amber-server ~/.local/share/amber-server   # settings and models
```

On Windows: `amber-server service uninstall`, then delete `%LOCALAPPDATA%\Programs\amber-server`,
`%APPDATA%\amber-server` and `%LOCALAPPDATA%\amber-server`.

## Building

```sh
cargo build -p amber-server --release                     # this machine's CPU
cargo build -p amber-server --release --features vulkan   # Vulkan (needs the Vulkan SDK)
sh server/test-install.sh                                  # the Linux installer against a fake release
```

whisper-rs needs cmake and libclang. Releases are built by `.github/workflows/server-release.yml`
on a `server-v<version>` tag; they never become GitHub's "latest" release (the desktop app's
updater reads that) and refresh the `server-latest` release the installers fetch from.
