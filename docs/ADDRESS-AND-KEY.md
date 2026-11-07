# Task: an address and a separate key, as with OpenAI

Fabi, 2026-11-07: connecting the phone to Amber PC or Amber Server should feel like OpenAI -
an address (a domain or an IP with its port) and, apart from it, an API key. Today the phone
gets one address with the key in its path (`http://<host>:<port>/<key>/v1`), and the QR code
holds exactly that.

## Why

- **One thing to type, one to keep secret.** An address can be read out, shown, written on a
  note; the key cannot. Mixed into one string, every place that shows the address shows the key.
- **The key leaves fewer traces.** A path ends up in Cloudflare's tunnel logs, browser
  history, proxy logs and crash reports; a header does not.
- **Amber for Android binds a key to the host it was entered for** (0.29.0): a changed address
  gets no key until it is typed again. That only works when the key is not part of the address.

## The contract (change `docs/phone-api.md` accordingly)

- Address: `http://<host>:<port>/v1` in the home network, `https://<tunnel-host>/v1` through
  the tunnel - no key in it.
- Key: `Authorization: Bearer <key>` on every request but `GET /health`. This already works
  (`api::route`); it becomes the documented way.
- The key in the path stays accepted for phones that still send it (Amber for Android up to
  0.29.0, and setups nobody has touched), but nothing shows that form any more. Removing it is
  a later decision, once Amber for Android has moved every stored address (it does that by
  itself from 0.30.0, see below).
- A pairing link carries both, for the QR code and for copying in one go:
  `amber://server?url=<address, percent-encoded>&key=<key>`. Amber for Android splits a pasted
  or scanned link into its two fields; a camera app shows it as text to copy.

## What to build

1. **`crates/core/src/api.rs`:** `phone_url` and `public_url` without the key; a new
   `pairing_link(url, key)` building the `amber://server?...` link (percent-encoding the
   address). Tests: the address has no key; the link round-trips; `route` still takes the key
   in the path and as Bearer (keep `routes_with_the_key_in_the_path_or_as_bearer`).
2. **Amber for Windows (`src-tauri/src/phone.rs`, the settings page):** per key the address
   and the key in two rows, each with its own copy button; the key masked until "Show". The QR
   code encodes the pairing link. Text: "Address" / "API key", as Amber for Android names them.
3. **Amber Server (`server/src/info.rs`):** the same two lines and the pairing link in its
   terminal output and QR code.
4. **`docs/phone-api.md`:** the section "Address and key" as above; the path form described as
   older and still accepted.
5. **Logs:** a request with the key in the path logs its path without the key (it may already;
   check), so moving clients over does not leave old keys in the log.

## What Amber for Android does (built there, 0.30.0)

- The Server page has "Address" and "API key" as two fields; pasting an old address with the
  key in it, or a pairing link, fills in both.
- A stored Amber server address with the key in its path is moved once: the key goes into the
  encrypted key store, bound to the host, and the address loses it - only for a server whose
  model list says it is Amber, so no other server is touched.
- The key always goes as `Authorization: Bearer`.
