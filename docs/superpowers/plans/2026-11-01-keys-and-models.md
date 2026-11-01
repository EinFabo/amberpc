# Access keys and the phone's model choice - plan

**Goal:** the phone (or any client) picks the speech and cleanup model on Amber PC as it already
can on Amber Server; models are still downloaded on the PC or server. Several access keys, each
with a name, the models it may use, its own daily quota, and revocable on its own.

**Fabi's decisions (2026-11-01):** the PC offers every downloaded model plus the cloud model set
in the app (when its key is there); one local model loaded at a time (a switch costs a reload);
a key has a name, allowed models, revocation and its own quota. Keys are advanced settings.

## Design

- `crates/core/src/api.rs`
  - `pub struct AccessKey { name, secret, models: Option<Vec<String>> /* None = all */, per_day: Option<u32> /* None = 2000 */ }`.
  - `Hooks.key` becomes `Hooks.keys: Fn() -> Vec<AccessKey>`. `route()` finds the key by the
    first path segment or the Bearer token (constant-time compare against each) and returns it
    with the route; no keys means nobody gets in.
  - `Guard` spends per key name: 120 per 10 minutes and the key's `per_day` (default 2000).
    Wrong-key pauses stay per sender.
  - `/v1/models` lists only what the key may use; per kind exactly one `default` (the server's
    default if allowed, else the first allowed of that kind).
  - Model resolution in the API, the same for PC and server: the request's `model` if the key
    may use it and it is listed, else the key's default of that kind; a kind the key may not use
    at all answers `403`. The engine hook gets the resolved id; the chat request gets it as
    `model`.
  - Log lines name the key: `phone API: 192.168.1.5 (Fabis Handy), ...`.
- Amber Server: config `[[keys]] name, key, models, per_day`; an old single `key` is taken as a
  key named "default". `amber-server key` lists; `key add <name> [--models a,b] [--per-day N]`,
  `key remove <name>`, `key new [<name>]` (a new secret), `url [--key <name>]`.
- Amber PC
  - `models()`: every downloaded local speech model and the cloud speech model set in the app (if
    its key is stored); every downloaded local cleanup model and the cloud cleanup model set in
    the app. Default = what the PC itself uses.
  - `engine(id)` / `chat(id)`: a local model id loads that model (one at a time, the PC's own
    dictations switch back on their next use); the cloud model goes to its provider; anything else
    is the PC's own.
  - Keys: names, models and quotas in the settings; secrets in the PC's secret store. The current
    key becomes "Phone" (same secret - the phone keeps working).
  - Phone page, advanced: the keys - name, address and QR, allowed models, quota, remove, add.
- `docs/phone-api.md`: keys and their models, `403`.

## Phases (each: TDD, the suites green, then a review)

1. **Core** - `AccessKey`, `route` with keys, per-key quota, filtered model list, model
   resolution, key name in the log. Tests: two keys reach the API, a revoked one does not; one
   key's quota does not touch the other's; the list shows only allowed models with one default
   per kind; a requested model the key may not use gets the key's default; a kind not allowed is
   403; an empty key list lets nobody in.
2. **Amber Server** - config with keys and its migration, the `key` commands, `url --key`,
   `status`/`config` showing names, not secrets. Tests: migration keeps the old secret; add,
   remove, new; names unique; models validated against the catalog.
3. **Amber PC backend** - keys in settings + secret store with migration; `models()` with all
   downloaded and the cloud ones; per-request model switching for speech and cleanup. Tests in
   core where logic can go wrong (the offered list from settings + installed models).
4. **Amber PC UI** - the keys section; check it by eye (`tools/screenshot.ps1`).
5. **Docs, review of the whole, release 0.4.4 and server 0.4.4.**
