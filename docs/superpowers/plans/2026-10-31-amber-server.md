# Amber Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A headless `amber-server` for Linux and Windows (CPU or Vulkan GPU) that serves Amber's phone API with downloadable speech and cleanup models, installs with one command, and has a command for every setting.

**Architecture:** A new workspace crate `server/` (binary `amber-server`) on top of `crates/core` (HTTP API, audio, rules, model catalogs, verified downloads, cleanup forwarding). Recognition runs whisper.cpp in-process (`whisper-rs`); the cleanup runs llama.cpp's own `llama-server` builds as a child process. A TOML config, a `clap` CLI, a per-user service, two install scripts and a release workflow complete it.

**Tech Stack:** Rust 2021, `amber-core` (path), `whisper-rs` 0.16 (feature `vulkan`), `clap` 4 (derive), `toml`, `serde`, `dirs`, `sysinfo`, `qrcode` (unicode), `if-addrs`, `reqwest` (blocking, as in core), POSIX sh + PowerShell 5.1, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-31-amber-server-design.md` (API contract: `docs/phone-api.md`)

## Global Constraints

- Everything in English: code, comments, logs, docs, CLI output, commit messages (CLAUDE.md).
- Local only: no cloud engine, no cloud key anywhere in config or CLI.
- The API is `amber_core::api::start_with` unchanged, `Hooks.service = "amber-server"`.
- Every download pinned and SHA-256 checked (`amber_core::local::download_verified`); installers check binaries against `SHA256SUMS` of the same release.
- llama.cpp build `local::LLAMA_CPP_BUILD` (b11981): `llama-b11981-bin-ubuntu-x64.tar.gz`, `llama-b11981-bin-ubuntu-vulkan-x64.tar.gz`, `llama-b11981-bin-win-cpu-x64.zip`, `llama-b11981-bin-win-vulkan-x64.zip` (SHA-256 from the release, fixed in the catalog).
- Builds: `{linux,windows}-x64-{cpu,cpu-compat,vulkan}`; `cpu-compat`: `GGML_NATIVE=OFF GGML_AVX=ON GGML_AVX2=OFF GGML_FMA=OFF GGML_F16C=OFF`; `cpu`: `GGML_NATIVE=OFF GGML_AVX2=ON GGML_FMA=ON GGML_F16C=ON`.
- Paths: config `dirs::config_dir()/amber-server/amber-server.toml` (mode 600 on Linux); data `dirs::data_local_dir()/amber-server/` with `models/`, `llm/`, `llama/<build>-<backend>/`.
- Default port 8765; llama-server on 127.0.0.1, a free port, a random `--api-key`.
- No secret is logged; only `url` and `key --new` print the key, to the terminal that asked.
- One speech model and one cleanup model in memory at a time.
- `server/Cargo.toml`: `version.workspace = true`; releases tagged `server-v<version>`.

## Review Focus

1. A CPU without AVX2 on the `cpu` build: a clear message naming `cpu-compat`, not "illegal instruction" - Task 3 (`cpu_check`), Task 9 picks the build.
2. The service starting before setup (no config or no model): one log line "run: amber-server setup" and exit 78, no restart loop - Task 6, Task 8 (`RestartPreventExitStatus=78`).
3. The phone naming a model that is not downloaded, or switching models back and forth: the default answers, no crash, the switch costs one reload - Task 2 and Task 4 tests.
4. A download cut off, or the disk full: a clean error, nothing half used - `download_verified` writes `.part`; Task 3 tests `model_ready` ignores `.part`.
5. llama-server dying or failing to start (old Vulkan driver): the chat route answers 503 with the reason, transcriptions keep working, the next chat request tries again after 60 s - Task 4.

---

### Task 1: Crate, config, CLI skeleton

**Files:** Create `server/Cargo.toml`, `server/src/main.rs`, `server/src/config.rs`; Modify `Cargo.toml` (members += `"server"`). Test in `config.rs`.

**Interfaces:**
- Produces: `pub struct Config { port: u16, key: String, speech: String, cleanup: Option<String>, language: String, vocabulary: String, threads: u32, gpu: Gpu, rules: RulesConfig }` (all `pub`, `#[serde(default)]`); `pub enum Gpu { Auto, Off }`; `pub struct RulesConfig { fillers: bool, spoken_punctuation: bool, german_commas: bool }` (default all true); `impl Config { fn path() -> PathBuf; fn load(&Path) -> Result<Config, String>; fn save(&self, &Path) -> Result<(), String>; fn sanitized(self) -> Config; fn get(&self, key: &str) -> Result<String, String>; fn set(&mut self, key: &str, value: &str) -> Result<(), String> }`; `pub fn data_dir() -> PathBuf`; `main.rs` with the full `clap` command tree of the spec (handlers `todo!()` until their task).

- [ ] **Step 1: Failing tests** - `round_trip_and_sanitize` (save/load equal; `port 80` -> 8765; unknown `speech` id -> the default `futo-multilingual-244`; unknown `cleanup` id -> `None`), `the_config_is_private` (Unix mode 600), `every_key_can_be_read_and_set` (`set("rules.fillers","false")` then `get` = `"false"`; `set("port","70000")` is `Err`; `set("nope","1")` is `Err` naming the valid keys).
- [ ] **Step 2: Run** `cargo test -p amber-server` - FAIL.
- [ ] **Step 3: Implement** (atomic save via temp file + rename; keys exactly those of the spec's `config` row).
- [ ] **Step 4: Run** - PASS; `cargo run -p amber-server -- --help` lists all commands.
- [ ] **Step 5: Commit** `Amber Server: crate, config, commands`

### Task 2: Recognition in-process, with model switching

**Files:** Create `server/src/speech.rs`. Modify `server/Cargo.toml` (`whisper-rs = "0.16"`, feature `vulkan = ["whisper-rs/vulkan"]`).

**Interfaces:**
- Consumes: `amber_core::engine::{Transcriber, TranscribeOptions, SttError, ErrorKind}`, `amber_core::audio::{decode_wav, resample, TARGET_RATE}`, `amber_core::local::{Model, model, audio_ctx_for}`.
- Produces: `pub struct Speech` (holds `Mutex<Option<(String, WhisperContext)>>`, the models dir, threads, gpu); `impl Speech { pub fn new(dir: PathBuf, threads: u32, gpu: bool) -> Speech; pub fn engine(self: &Arc<Self>, requested: Option<&str>, default: &str) -> Result<Box<dyn Transcriber>, String> }` - the requested id if downloaded, else the default; loads it if another is loaded. `fn params(options: &TranscribeOptions, samples: usize, m: &Model, threads: u32) -> ParamsPlan` with `struct ParamsPlan { language: Option<String>, prompt: Option<String>, audio_ctx: u32, threads: i32, no_timestamps: bool }`.

- [ ] **Step 1: Failing tests** - `params_follow_the_model_and_the_request` (tuned model: `0 < audio_ctx < 1500`; stock model: 0; `no_timestamps`; language/prompt passed), `an_unknown_or_missing_model_gets_the_default` (with a temp dir holding only the default's file name: `chosen(Some("large-v3-q5_0"))` = default, `chosen(Some("<default>"))` = default, `chosen(None)` = default - test the pure `fn chosen(dir, requested, default) -> String`), `speech_is_shared_across_threads` (`Send + Sync`), and `#[ignore]` `ten_minutes_of_silence` (ggml-tiny, 600 s zeros: Ok in under 60 s).
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement** (greedy, best_of 1; segment texts joined; whisper errors -> `ErrorKind::Server`). **Step 4: Run** - PASS (+ `--ignored` once).
- [ ] **Step 5: Commit** `Amber Server: recognition in-process`

### Task 3: Catalogs, downloads, `models` and `cleanup` commands

**Files:** Create `server/src/catalog.rs`, `server/src/cpu.rs`; Modify `crates/core/src/local.rs` (add `pub const LLAMA_SERVER_BUILDS: [LlamaBuild; 4]` with `struct LlamaBuild { os: &'static str, backend: &'static str, url, sha256, size_mb, exe: &'static str }` for the four archives of Global Constraints; tar.gz extraction next to the zip one).

**Interfaces:**
- Produces: `pub fn model_ready(dir: &Path, m: &Model) -> bool`; `pub fn llm_ready(dir: &Path, m: &LlmModel) -> bool`; `pub fn fit(m: &Model, gpu: bool, threads: u32, ram_mb: u64) -> &'static str` ("recommended" / "fits" / "slow here" / "too big"); `pub fn recommended_speech(gpu: bool, threads: u32, ram_mb: u64) -> &'static str` (via `local::recommend(..).1`); `pub fn download_speech(id) / download_cleanup(id)` with a progress bar on stderr; `pub fn ensure_llama(data: &Path, gpu: bool) -> Result<PathBuf, String>` (the llama-server exe, downloaded and unpacked once); `cpu::cpu_check() -> Result<(), String>`; handlers for `models [add|remove|default]` and `cleanup [add|remove|default|off]` (removing the default is refused with a hint).

- [ ] **Step 1: Failing tests** - `a_half_download_is_no_model` (`.part` only -> false), `fit_tells_the_machine_apart` (`large-v3-q5_0` on 4 threads/4 GB without GPU -> "too big"; `futo-multilingual-74` there -> "recommended"), `the_llama_builds_cover_both_systems` (each of `linux/cpu`, `linux/vulkan`, `windows/cpu`, `windows/vulkan` once; urls start with `https://github.com/ggml-org/llama.cpp/releases/download/b11981/`; 64-hex sha256), `the_default_cannot_be_removed`.
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement.** **Step 4: Run** - PASS; manual `amber-server models` prints both lists.
- [ ] **Step 5: Commit** `Amber Server: models and cleanup commands`

### Task 4: The cleanup's llama-server

**Files:** Create `server/src/cleanup.rs`, `server/src/child.rs` (spawn with stdout/stderr kept; Linux: `prctl(PR_SET_PDEATHSIG, SIGTERM)` via `libc` in `pre_exec`; Windows: a kill-on-close job object - as `src-tauri/src/process.rs`).

**Interfaces:**
- Consumes: `ensure_llama`, `llm_ready`, `amber_core::cleanup::ChatCleaner::{new, forward}`, `amber_core::local::{llm_model, LlmModel}`.
- Produces: `pub struct Cleanup` with `pub fn answer(&self, request: &serde_json::Value, default: &str) -> Result<serde_json::Value, (u16, String)>` - picks `request["model"]` if downloaded else the default; starts or restarts llama-server for it (`--model <file> --host 127.0.0.1 --port <free> --api-key <random> --ctx-size 4096 --parallel 1`), waits for `/health` (180 s), forwards with `ChatCleaner`; after a failed start answers 503 with the last output lines for 60 s before trying again.

- [ ] **Step 1: Failing tests** - with a stub "llama-server" (a tiny Rust test binary in `server/tests/bin/fake_llama.rs` that answers `/health` and `/v1/chat/completions` with the model name it was started with): `chat_starts_the_model_once_and_switches_on_request` (two requests for A: one start; one for B: a restart; reply names B), `a_model_not_downloaded_gets_the_default`, `a_failing_start_answers_503_and_waits` (stub exits at once: 503 with its output; a second call within 60 s does not respawn).
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement** (the exe path injectable for the tests). **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** `Amber Server: cleanup through llama-server`

### Task 5: setup - quick and asking

**Files:** Create `server/src/setup.rs`.

**Interfaces:**
- Consumes: Tasks 1-3.
- Produces: `pub fn quick(path: &Path) -> Result<Config, String>` (keeps an existing key, else `api::new_key()`; the recommended speech model downloaded; no cleanup); `pub fn ask(path: &Path, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<Config, String>` (speech model from a numbered list with the fit column, cleanup none or one of `LLM_MODELS`, language, port - Enter takes the shown default).

- [ ] **Step 1: Failing tests** - `asking_with_enter_everywhere_equals_quick` (input `"\n\n\n\n"` with downloads stubbed: same speech model, no cleanup, port 8765), `asking_takes_the_numbers_given` (input picks list entry 2 and cleanup 1: config holds those ids), `setup_keeps_the_key`.
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement** (downloads behind a `trait Fetch` so tests stub them). **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** `Amber Server: setup`

### Task 6: run - serving

**Files:** Create `server/src/serve.rs`.

**Interfaces:**
- Consumes: `Config`, `Speech`, `Cleanup`, `amber_core::api::{start_with, Hooks, Limits, ModelInfo, ModelKind}`, `amber_core::text::{by_rule, Rules}`.
- Produces: `pub fn hooks(config: &Config, speech: Arc<Speech>, cleanup: Arc<Cleanup>) -> Hooks` (`models`: every downloaded speech and cleanup model, label from the catalog, note "<size> · CPU|GPU", the config's defaults marked; `chat` -> `Cleanup::answer`, 503 "no cleanup model - amber-server cleanup add" when none); `pub fn serve(config: Config) -> Result<(), String>` (exit 78 without config or model; `cpu_check`; `AddrInUse` -> "port N is in use - amber-server config set port <other>").

- [ ] **Step 1: Failing tests** - `the_model_list_holds_what_is_downloaded` (temp data dir with two speech files and one LLM file: three entries, kinds right, one default per kind), `chat_without_a_cleanup_model_says_how_to_get_one` (503, message contains "amber-server cleanup add"), `the_rules_from_the_config_apply` (`"ähm hallo"` -> `"Hallo"` with fillers on, unchanged off).
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement.** **Step 4: Run** - PASS; manual: `amber-server run`, then `curl -s localhost:8765/health` shows `"service":"amber-server"`.
- [ ] **Step 5: Commit** `Amber Server: run`

### Task 7: url, key, config, status, logs

**Files:** Create `server/src/info.rs`.

**Interfaces:**
- Produces: `pub fn describe(config: &Config, interfaces: &[(String, IpAddr)]) -> String` (addresses via `api::phone_addresses`/`phone_url`, QR of the first in `Dense1x2` blocks); `key --new`; `config` (all keys and values, the key masked except its last 4); `status` (config path, default models and whether ready, service state, `GET /health` of the running server or "not running"); `logs` (`journalctl --user -u amber-server -n 50` / the task's log file).

- [ ] **Step 1: Failing tests** - `lists_the_home_address_first_with_its_code` (virtual adapters left out; block characters present), `config_never_shows_the_whole_key`.
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement.** **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** `Amber Server: url, key, config, status, logs`

### Task 8: The background service

**Files:** Create `server/src/service.rs`.

**Interfaces:**
- Produces: `pub fn systemd_unit(exe: &Path) -> String`; `pub fn schtasks_args(exe: &Path, log: &Path) -> Vec<String>`; `install()`, `uninstall()`, `state() -> String`.

- [ ] **Step 1: Failing tests** - `the_unit_restarts_on_failure_but_not_without_setup` (`ExecStart=<exe> run`, `Restart=on-failure`, `RestartPreventExitStatus=78`, `WantedBy=default.target`), `the_task_starts_at_logon_without_admin_rights` (`/sc onlogon`, `/rl limited`, output redirected to the log file).
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement** (Linux: write the unit, `systemctl --user daemon-reload`, `enable --now`, `loginctl enable-linger $USER` or print the sudo line; Windows: `schtasks` + start once). **Step 4: Run** - PASS; manual in WSL with systemd: `systemctl --user status amber-server` active.
- [ ] **Step 5: Commit** `Amber Server: service`

### Task 9: One-command installers

**Files:** Create `server/install-server.sh`, `server/install-server.ps1`, `server/test-install.sh`.

- [ ] **Step 1: Failing test** - `server/test-install.sh`: a fake release folder (`AMBER_SERVER_BASE=file://...`) with a stub `amber-server-linux-x64-cpu-compat` recording its arguments and a `SHA256SUMS`; with `AMBER_SERVER_FORCE_BUILD=cpu-compat HOME=$tmp`: the binary lands in `$tmp/.local/bin/amber-server`, the stub saw `setup --quick`, `service install`, `url` in that order; a wrong checksum: non-zero exit, nothing installed.
- [ ] **Step 2: Run** `sh server/test-install.sh` - FAIL.
- [ ] **Step 3: Implement** - sh: `vulkan` if `vulkaninfo --summary` lists a GPU, else `cpu` with `avx2` in `/proc/cpuinfo`, else `cpu-compat`; curl or wget; `sha256sum -c`; the three commands; the closing hint about `amber-server cleanup add`. PowerShell: the same (Vulkan: `vulkan-1.dll` and a GPU in `Win32_VideoController`; AVX2: `IsProcessorFeaturePresent(40)`), user PATH updated.
- [ ] **Step 4: Run** - PASS; `shellcheck server/install-server.sh` clean.
- [ ] **Step 5: Commit** `Amber Server: installers`

### Task 10: Release workflow

**Files:** Create `.github/workflows/server-release.yml`.

- [ ] **Step 1:** Matrix `{ubuntu-22.04, windows-2022} x {cpu, cpu-compat, vulkan}`; Vulkan SDK via `humbletim/setup-vulkan-sdk` pinned by commit; CPU flags via `CMAKE_ARGS`/`WHISPER_*` env for whisper-rs-sys; `cargo build -p amber-server --release [--features vulkan]`; artifact `amber-server-<os>-x64-<build>[.exe]`.
- [ ] **Step 2:** A final job writes `SHA256SUMS` and runs `gh release create server-v<version>` with the six binaries, the sums and both installers.
- [ ] **Step 3: Verify** with a `server-v0.0.1-test` tag on a branch: 10 files; `amber-server-linux-x64-cpu-compat --version` runs in WSL; delete the test release and tag.
- [ ] **Step 4: Commit** `Amber Server: release workflow`

### Task 11: Documentation

**Files:** Create `server/README.md`; Modify `README.md` (a short "Amber Server" section).

- [ ] **Step 1:** Install commands, what the installer does, every command with an example, `amber-server.toml` with all keys, adding a cleanup model, choosing models on the phone, firewall (`ufw allow 8765/tcp`; Windows asks on first run), uninstalling.
- [ ] **Step 2: Commit** `Amber Server: docs`
