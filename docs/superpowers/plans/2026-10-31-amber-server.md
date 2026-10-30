# Amber Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A headless `amber-server` binary for Linux and Windows (CPU or Vulkan GPU) that serves Amber's phone API and installs with one command.

**Architecture:** A new workspace crate `server/` (binary `amber-server`) that reuses `crates/core` for the HTTP API, audio, rules, cleanup forwarding, model catalog and verified downloads, and adds an in-process whisper.cpp transcriber (`whisper-rs`), a TOML config, a small CLI, per-user service install, install scripts and a release workflow.

**Tech Stack:** Rust 2021, `amber-core` (path dependency), `whisper-rs` 0.16 (features `vulkan` for the GPU build), `clap` 4 (derive), `toml` + `serde`, `dirs` (config and data folders), `sysinfo` (RAM), `qrcode` (unicode renderer), `if-addrs`, POSIX sh + PowerShell 5.1 for the installers, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-31-amber-server-design.md`

## Global Constraints

- Everything in English: code, comments, logs, docs, commit messages (CLAUDE.md).
- No change to the API's behaviour: the server calls `amber_core::api::start_with` - routes, key check, private-peer check and limits stay as they are in `crates/core`.
- Every download is pinned and SHA-256 checked (`amber_core::local::download_verified`); the installers check binaries against `SHA256SUMS` from the same release.
- Builds: `linux-x64-{cpu,cpu-compat,vulkan}`, `windows-x64-{cpu,cpu-compat,vulkan}`; `cpu-compat` is built with `GGML_NATIVE=OFF GGML_AVX=ON GGML_AVX2=OFF GGML_FMA=OFF GGML_F16C=OFF`, `cpu` with `GGML_NATIVE=OFF GGML_AVX2=ON GGML_FMA=ON GGML_F16C=ON`.
- Config `amber-server.toml` in `dirs::config_dir()/amber-server/`, mode 600 on Linux; models in `dirs::data_local_dir()/amber-server/models/`.
- Default bind `0.0.0.0:8765` (`amber_core::api::DEFAULT_PORT`).
- No secret (access key, cloud key) is ever logged; `url` prints the address with the key only to the terminal that asked.
- Version: `server/Cargo.toml` uses `version.workspace = true`; release tags are `server-v<version>`.

## Review Focus

1. A CPU without AVX2 (a Sandy Bridge office PC) running the `cpu` build: expected a clear message naming `cpu-compat`, not a crash with "illegal instruction" - Task 3 checks the CPU at start; Task 7's installer picks `cpu-compat`.
2. The service starting before `setup` ran (no config, no model): expected one log line "run amber-server setup" and exit code 78, no restart loop hammering the disk - Task 4 and Task 6 (`RestartPreventExitStatus=78`).
3. The port already taken, or the firewall blocking the LAN: expected "port 8765 is in use - set port in amber-server.toml" and, on Windows, the hint that the first run asks for the firewall - Task 4.
4. A model download cut off or the disk full: expected a clean error and no half model used later - `download_verified` writes `.part` first; Task 3 tests that `setup` refuses a model path that exists only as `.part`.
5. Two devices transcribing at once, or a 10-minute recording: expected both answered in turn, nothing crashes - `api.rs` already serializes; Task 2 tests that the transcriber is `Send + Sync` and handles 600 s of audio.

---

### Task 1: Crate skeleton and config

**Files:**
- Create: `server/Cargo.toml`, `server/src/main.rs`, `server/src/config.rs`
- Modify: `Cargo.toml` (workspace `members` gets `"server"`)
- Test: `server/src/config.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub struct Config { pub port: u16, pub key: String, pub engine: Engine, pub model: String, pub threads: u32, pub language: String, pub vocabulary: String, pub rules: RulesConfig, pub cloud: Option<Remote>, pub chat: Option<Remote> }`
  - `pub enum Engine { Local, Cloud }` (serde lowercase)
  - `pub struct RulesConfig { pub fillers: bool, pub spoken_punctuation: bool, pub german_commas: bool }` (all default `true`)
  - `pub struct Remote { pub base_url: String, pub api_key: String, pub model: String }`
  - `impl Config { pub fn path() -> PathBuf; pub fn load(path: &Path) -> Result<Config, String>; pub fn save(&self, path: &Path) -> Result<(), String>; pub fn sanitized(self) -> Config }`
  - `pub fn models_dir() -> PathBuf`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn defaults_round_trip_and_sanitize() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("amber-server.toml");
    let c = Config { key: "k".repeat(20), ..Config::default() };
    c.save(&path).unwrap();
    assert_eq!(Config::load(&path).unwrap(), c);
    assert_eq!(Config::default().port, 8765);
    assert!(Config::default().rules.fillers);
    let bad = Config { port: 80, model: "gone".into(), ..Config::default() }.sanitized();
    assert_eq!(bad.port, 8765);
    assert_eq!(bad.model, Config::default().model);
}

#[cfg(unix)]
#[test]
fn the_config_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("amber-server.toml");
    Config::default().save(&path).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}
```

- [ ] **Step 2: Run them to see them fail** - `cargo test -p amber-server config` - FAIL (crate does not exist).
- [ ] **Step 3: Implement** `Config` with `#[serde(default)]`, defaults: `engine Local`, `model "futo-multilingual-244"`, `threads 0` (= all cores), `language "auto"`; `sanitized` resets an unknown model (`amber_core::local::model`) and ports below 1024; `save` writes atomically (temp file + rename) and sets mode 600 on Unix. `main.rs` only declares modules for now.
- [ ] **Step 4: Run** `cargo test -p amber-server` - PASS.
- [ ] **Step 5: Commit** - `git commit -m "Amber Server: crate and config"`

### Task 2: In-process whisper transcriber

**Files:**
- Create: `server/src/whisper.rs`
- Modify: `server/Cargo.toml` (`whisper-rs = "0.16"`, feature `vulkan = ["whisper-rs/vulkan"]`)
- Test: `server/src/whisper.rs`

**Interfaces:**
- Consumes: `amber_core::engine::{Transcriber, TranscribeOptions, SttError, ErrorKind}`, `amber_core::audio::{decode_wav, resample, TARGET_RATE}`, `amber_core::local::{Model, audio_ctx_for}`
- Produces: `pub struct LocalWhisper` with `pub fn load(model_path: &Path, model: &'static Model, threads: u32) -> Result<LocalWhisper, String>`; `impl Transcriber for LocalWhisper` (one `WhisperContext`, a `Mutex<WhisperState>` per call); `fn params(options: &TranscribeOptions, samples: usize, model: &Model, threads: u32) -> ParamsPlan` where `struct ParamsPlan { language: Option<String>, prompt: Option<String>, audio_ctx: u32, threads: i32, no_timestamps: bool }`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn params_follow_the_model_and_the_request() {
    let tuned = amber_core::local::model("futo-multilingual-244").unwrap();
    let p = params(&TranscribeOptions::new("de", "Tauri"), 16_000 * 3, tuned, 4);
    assert_eq!(p.language.as_deref(), Some("de"));
    assert_eq!(p.prompt.as_deref(), Some("Tauri"));
    assert!(p.audio_ctx > 0 && p.audio_ctx < 1500, "a tuned model stops where the speech stops");
    assert!(p.no_timestamps);
    let stock = amber_core::local::model("large-v3-turbo-q5_0").unwrap();
    assert_eq!(params(&TranscribeOptions::new("auto", ""), 16_000 * 3, stock, 4).audio_ctx, 0);
}

#[test]
fn the_transcriber_can_be_shared() {
    fn shared<T: Send + Sync>() {}
    shared::<LocalWhisper>();
}

#[test]
#[ignore = "downloads ggml-tiny (75 MB): cargo test -p amber-server -- --ignored"]
fn recognizes_ten_minutes_of_silence_without_trouble() {
    // ggml-tiny from the pinned catalog URL pattern, 600 s of zeros as a WAV:
    // Ok with empty or whitespace text, in under 60 s on a CPU.
}
```

- [ ] **Step 2: Run** `cargo test -p amber-server whisper` - FAIL.
- [ ] **Step 3: Implement** `params` (audio_ctx via `audio_ctx_for(samples, model.context_slack)` when `model.short_context`, else 0; `threads 0` -> `std::thread::available_parallelism`), then `LocalWhisper`: decode the WAV with `decode_wav`, `resample` to 16 kHz, run `full` with `SamplingStrategy::Greedy { best_of: 1 }`, join segment texts. A whisper error maps to `ErrorKind::Server`.
- [ ] **Step 4: Run** `cargo test -p amber-server` - PASS; once `cargo test -p amber-server -- --ignored` - PASS.
- [ ] **Step 5: Commit** - `git commit -m "Amber Server: whisper.cpp in-process"`

### Task 3: setup - hardware, model, key

**Files:**
- Create: `server/src/setup.rs`, `server/src/cpu.rs`
- Test: both files

**Interfaces:**
- Consumes: `Config` (Task 1), `amber_core::local::{recommend, MODELS, download_verified}`, `amber_core::api::new_key`
- Produces: `pub fn run_setup(path: &Path, choose_model: Option<String>) -> Result<Config, String>`; `pub fn model_ready(dir: &Path, model: &Model) -> bool` (the file is there with the catalog size - a `.part` does not count); `pub fn cpu_check() -> Result<(), String>` (in `cpu.rs`: when built without `vulkan` and with AVX2 enabled, `Err("This CPU has no AVX2 - install the cpu-compat build: ...")` if `is_x86_feature_detected!("avx2")` is false); `pub fn pick_model(gpu: bool, threads: u32, ram_mb: u64) -> &'static str`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn picks_a_model_the_machine_can_carry() {
    assert_eq!(pick_model(false, 4, 3_900), "futo-multilingual-74");
    assert_eq!(pick_model(true, 16, 32_000), "acft-large-v3-turbo-q8_0");
}

#[test]
fn a_half_download_is_no_model() {
    let dir = tempfile::tempdir().unwrap();
    let m = amber_core::local::model("futo-multilingual-74").unwrap();
    std::fs::write(dir.path().join(m.file).with_extension("part"), b"x").unwrap();
    assert!(!model_ready(dir.path(), m));
}
```

- [ ] **Step 2: Run** - FAIL.
- [ ] **Step 3: Implement** `pick_model` with `recommend(gpu, threads, ram_mb).1` (the Vulkan build passes `gpu = true` when Vulkan finds a device, RAM from `sysinfo`); `run_setup` keeps an existing key, creates one with `new_key()` otherwise, downloads the model with a progress bar on stderr (`download_verified`), saves the config and prints "Ready - run: amber-server url".
- [ ] **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** - `git commit -m "Amber Server: setup"`

### Task 4: run - serving the API

**Files:**
- Create: `server/src/serve.rs`
- Modify: `server/src/main.rs` (clap: `setup`, `run`, `url`, `key --new`, `service install|uninstall`, `status`)
- Test: `server/src/serve.rs`

**Interfaces:**
- Consumes: `Config`, `LocalWhisper`, `amber_core::api::{start_with, Hooks, Limits}`, `amber_core::engine::OpenAiTranscriber::new(base_url, api_key, model)`, `amber_core::cleanup::ChatCleaner::new(base_url, api_key, model, None)`, `amber_core::text::{by_rule, Rules}`
- Produces: `pub fn hooks(config: &Config, engine: Arc<dyn Transcriber>) -> Hooks`; `pub fn serve(config: Config) -> Result<(), String>` (blocks); exit code 78 (`EX_CONFIG`) when there is no config or model.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn chat_without_a_configured_endpoint_says_so() {
    let h = hooks(&Config::default(), Arc::new(Silent));
    let err = (h.chat)(&serde_json::json!({ "messages": [{ "role": "user", "content": "x" }] })).unwrap_err();
    assert_eq!(err.0, 503);
    assert!(err.1.contains("chat"));
}

#[test]
fn the_rules_from_the_config_apply() {
    let mut c = Config::default();
    c.rules.fillers = true;
    assert_eq!((hooks(&c, Arc::new(Silent)).tidy)("ähm hallo".into()), "Hallo");
    c.rules.fillers = false;
    assert_eq!((hooks(&c, Arc::new(Silent)).tidy)("ähm hallo".into()), "ähm hallo");
}
```

(`Silent` is a test `Transcriber` returning `Ok(String::new())`.)

- [ ] **Step 2: Run** - FAIL.
- [ ] **Step 3: Implement** `hooks` (key from the config; `record` a no-op; `log` to stdout with a timestamp), `serve` (load and sanitize the config; `cpu_check()`; engine: `LocalWhisper::load` or `OpenAiTranscriber`; `start_with("0.0.0.0:<port>", hooks, Limits::default())`; `AddrInUse` -> "port N is in use - set port in amber-server.toml"; then park the main thread). `status` prints the config path, whether the model is ready (`model_ready`), and the answer of `GET http://127.0.0.1:<port>/health` (or "not running").
- [ ] **Step 4: Run** - PASS. Manual: `cargo run -p amber-server -- run` with a config in a temp `XDG_CONFIG_HOME`, then `curl -s localhost:8765/health` returns `{"status":"ok",...}`.
- [ ] **Step 5: Commit** - `git commit -m "Amber Server: run"`

### Task 5: url and key - the phone's address in the terminal

**Files:**
- Create: `server/src/url.rs`
- Test: `server/src/url.rs`

**Interfaces:**
- Consumes: `amber_core::api::{phone_addresses, phone_url, new_key}`, `if_addrs`, `qrcode` (`render::unicode::Dense1x2`)
- Produces: `pub fn describe(config: &Config, interfaces: &[(String, IpAddr)]) -> String` (each address on its own line, the QR code of the first); `key --new` replaces the key, saves, and prints the new address.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn lists_the_home_address_first_with_its_code() {
    let c = Config { key: "abc".into(), ..Config::default() };
    let out = describe(&c, &[("WSL".into(), "172.20.0.2".parse().unwrap()), ("Ethernet".into(), "192.168.1.20".parse().unwrap())]);
    assert!(out.contains("http://192.168.1.20:8765/abc/v1"));
    assert!(!out.contains("172.20.0.2"), "virtual adapters are left out");
    assert!(out.contains('▀') || out.contains('▄'), "a QR code in block characters");
}
```

- [ ] **Step 2: Run** - FAIL. **Step 3: Implement.** **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** - `git commit -m "Amber Server: url and key"`

### Task 6: service install

**Files:**
- Create: `server/src/service.rs`
- Test: `server/src/service.rs`

**Interfaces:**
- Produces: `pub fn systemd_unit(exe: &Path) -> String`; `pub fn schtasks_args(exe: &Path) -> Vec<String>`; `pub fn install() -> Result<(), String>`; `pub fn uninstall() -> Result<(), String>`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_unit_restarts_on_failure_but_not_without_setup() {
    let unit = systemd_unit(Path::new("/home/a/.local/bin/amber-server"));
    assert!(unit.contains("ExecStart=/home/a/.local/bin/amber-server run"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(unit.contains("RestartPreventExitStatus=78"));
    assert!(unit.contains("WantedBy=default.target"));
}

#[test]
fn the_task_starts_at_logon_without_admin_rights() {
    let args = schtasks_args(Path::new(r"C:\Users\a\AppData\Local\Programs\amber-server\amber-server.exe"));
    assert!(args.windows(2).any(|w| w == ["/sc", "onlogon"]));
    assert!(args.windows(2).any(|w| w == ["/rl", "limited"]));
    assert!(args.iter().any(|a| a.ends_with("amber-server.exe\" run")));
}
```

- [ ] **Step 2: Run** - FAIL.
- [ ] **Step 3: Implement** `install`: Linux writes `~/.config/systemd/user/amber-server.service`, runs `systemctl --user daemon-reload` and `systemctl --user enable --now amber-server`, and `loginctl enable-linger $USER` (on failure prints the sudo command); Windows runs `schtasks` with the args and starts the task once.
- [ ] **Step 4: Run** - PASS. Manual: in WSL with systemd, `amber-server service install`, then `systemctl --user status amber-server` shows active.
- [ ] **Step 5: Commit** - `git commit -m "Amber Server: service"`

### Task 7: The one-command installers

**Files:**
- Create: `server/install-server.sh`, `server/install-server.ps1`
- Test: `server/test-install.sh` (runs the sh installer against a local fake release folder via `AMBER_SERVER_BASE=file://...`)

**Interfaces:**
- Consumes: release assets `amber-server-<os>-x64-<build>[.exe]` and `SHA256SUMS` (Task 8)
- Produces: the installed binary, the config, the running service, the address and QR code on screen.

- [ ] **Step 1: Write the failing test** - `server/test-install.sh` builds a fake release folder with a stub `amber-server-linux-x64-cpu-compat` (a shell script that records its arguments) and a `SHA256SUMS`, runs `AMBER_SERVER_BASE=file://$fake AMBER_SERVER_FORCE_BUILD=cpu-compat HOME=$tmp sh install-server.sh`, and asserts: the binary is in `$tmp/.local/bin/amber-server`, the stub was called with `setup` then `service install` then `url`; a corrupted `SHA256SUMS` line makes the script exit non-zero without installing.
- [ ] **Step 2: Run** `sh server/test-install.sh` - FAIL.
- [ ] **Step 3: Implement** `install-server.sh` (POSIX sh, `set -eu`): build choice - `vulkan` if `vulkaninfo --summary` lists a GPU, else `cpu` if `/proc/cpuinfo` has `avx2`, else `cpu-compat`; download with curl or wget; `sha256sum -c`; install; run the three commands. `install-server.ps1` does the same on Windows (Vulkan: `vulkan-1.dll` present and an NVIDIA/AMD/Intel GPU in `Win32_VideoController`; AVX2 via `IsProcessorFeaturePresent(40)`), installs to `%LOCALAPPDATA%\Programs\amber-server`, and adds that folder to the user PATH.
- [ ] **Step 4: Run** `sh server/test-install.sh` - PASS; `shellcheck server/install-server.sh` - clean.
- [ ] **Step 5: Commit** - `git commit -m "Amber Server: one-command installers"`

### Task 8: Release workflow

**Files:**
- Create: `.github/workflows/server-release.yml`

**Interfaces:**
- Produces: on a `server-v*` tag, a GitHub release with the six binaries, `SHA256SUMS`, `install-server.sh`, `install-server.ps1`.

- [ ] **Step 1:** Matrix `{ubuntu-22.04, windows-2022} x {cpu, cpu-compat, vulkan}`; the Vulkan jobs install the Vulkan SDK (`humbletim/setup-vulkan-sdk`, pinned by commit); the CPU flags from Global Constraints go in through `WHISPER_*`/`CMAKE_ARGS` env that whisper-rs-sys passes to CMake; `cargo build -p amber-server --release [--features vulkan]`; rename to `amber-server-<os>-x64-<build>[.exe]`.
- [ ] **Step 2:** A final job collects the files, writes `SHA256SUMS` (`sha256sum *`), and creates the release with `gh release create`.
- [ ] **Step 3: Verify** - push a `server-v0.0.1-test` tag on a branch; the release has 10 files; download `amber-server-linux-x64-cpu-compat`, run `--version` in WSL; delete the test release and tag.
- [ ] **Step 4: Commit** - `git commit -m "Amber Server: release workflow"`

### Task 9: Documentation

**Files:**
- Create: `server/README.md`
- Modify: `README.md` (a short "Amber Server" section linking to it)

- [ ] **Step 1:** `server/README.md`: the two install commands, what the installer does, the commands, the config keys with an example `amber-server.toml`, how to point Amber for Android at it, firewall notes (Windows asks on first run; `ufw allow 8765/tcp` on Linux), how to uninstall (`amber-server service uninstall`, delete the folders).
- [ ] **Step 2: Commit** - `git commit -m "Amber Server: docs"`
