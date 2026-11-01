# Amber's phone API

What Amber for Windows (`service: "amber-pc"`) and Amber Server (`service: "amber-server"`)
offer to Amber for Android and to any OpenAI-compatible client. The implementation is
`crates/core/src/api.rs`; this page is the contract the phone app builds on.

## Address and key

`http://<host>:<port>/<key>/v1` in the home network, `https://<tunnel-host>/<key>/v1` through
a Cloudflare tunnel. The key is part of the path; `Authorization: Bearer <key>` works too.
Without the key: `401`. Ten wrong keys from one sender: `429` for 15 minutes. With the key:
120 requests per 10 minutes and 2000 a day, then `429`.

## Endpoints

### `GET /health` (no key)

```json
{ "status": "ok", "service": "amber-pc", "version": "0.4.2" }
```

### `GET /v1/models`

An OpenAI model list. Each entry carries an `amber` object, and the top level an `amber`
object - **that top-level object is how a client knows it talks to Amber**; any other server
lacks it.

```json
{
  "object": "list",
  "data": [
    {
      "id": "acft-large-v3-turbo-q8_0", "object": "model", "owned_by": "amber",
      "amber": { "kind": "speech", "label": "Large v3 Turbo", "note": "on this PC · NVIDIA GPU (CUDA)", "default": true }
    },
    {
      "id": "qwen3-4b-instruct-2507-q4km", "object": "model", "owned_by": "amber",
      "amber": { "kind": "cleanup", "label": "Qwen3 4B Instruct", "note": "on this PC · NVIDIA GPU (CUDA)", "default": true }
    }
  ],
  "amber": { "service": "amber-pc", "version": "0.4.2" }
}
```

- `kind`: `"speech"` (for `/v1/audio/transcriptions`) or `"cleanup"` (for
  `/v1/chat/completions`).
- `label`, `note`: for display - the name and one short line (where it runs, speed, size).
- `default`: what a request gets that names no model or an unknown one. Exactly one per kind.
- A kind may be missing: no `cleanup` entry means this server offers no cleanup right now
  (then `/v1/chat/completions` answers `503`).
- Amber for Windows lists the one speech and the one cleanup model it uses. Amber Server lists
  every model that is downloaded, and switches to the one a request names.
- Top-level `amber.audio` (optional): the formats `/v1/audio/transcriptions` takes, e.g.
  `["wav", "opus"]` - see [Compressed uploads](#compressed-uploads). Missing means `["wav"]`.

### `POST /v1/audio/transcriptions`

Multipart form: `file` (WAV, any rate, 1-8 channels, up to 15 min - or a compressed format the
server lists in `amber.audio`), optional `model` (an id of
kind `speech`), `language`, `prompt` (names and terms), `amber_cleanup`. Answer:
`{ "text": "..." }`, already cleaned by the server's rules (fillers, spoken punctuation, German
commas) and, on Amber for Windows, its cleanup model - as its owner set them. Silence answers
`{ "text": "" }`.

- `language`: `auto`, one ISO 639-1 code (`de`; everything else is translated into it), or
  several, comma-separated (`de,en`): the server detects which of them was spoken, per
  dictation - a short German dictation is then not taken for Turkish or translated into
  English, as `auto` can. On a tie, or when it heard none of them, the first wins.
- `amber_cleanup=client`: the client cleans the text up itself, with its own prompt
  (`/v1/chat/completions`). The server then leaves out its own rules and cleanup and returns
  what was recognized (Whisper's annotations such as `[Musik]` removed).

### `POST /v1/chat/completions`

An OpenAI chat request, at most 256 KB. `messages`, `temperature` and `max_tokens` (at most
4096, 2048 if none) are used; `model` picks a model of kind `cleanup` (Amber Server; Amber for
Windows always uses its own); other fields are dropped - they belong to the caller's idea of a
model, not to the server's. The answer is the model's OpenAI answer as it came.

## Compressed uploads

A phone on mobile data spends more time uploading than the server spends recognizing: WAV at
16 kHz is 32 KB per second of speech, so a 20-second dictation is 640 KB - several seconds on a
weak uplink, and the server's log shows it (`... (640 KB uploaded in 7200 ms)`). Opus at
24 kbit/s is about a tenth of that and loses nothing whisper needs.

- A server that takes more than WAV says so in `/v1/models`:
  `"amber": { "service": "...", "version": "...", "audio": ["wav", "opus"] }`.
- `"opus"`: Opus in an Ogg container (RFC 7845), mono, any of Opus' rates; sent as
  `file` with the filename `dictation.ogg` and `Content-Type: audio/ogg`. Recommended:
  16 kHz input, 24 kbit/s, 20 ms frames.
- The client takes the first format in its own order of preference that the list holds, and
  WAV otherwise; values it does not know it ignores (a later `"flac"`, say).
- The server tells the formats apart by their first bytes (`RIFF`, `OggS`), not by the
  filename. A format it does not take: `400` with the message, as for broken WAV.
- Whatever came in, the server resamples to 16 kHz mono itself - the rest of the request is
  the same.

Amber for Windows from 0.4.2 and Amber Server take `"opus"` (decoded by libopus,
`crates/core/src/audio.rs`) and list it.

## Errors

`{ "error": { "message": "...", "type": "..." } }` with `400` (bad request), `401` (key),
`404` (unknown route), `411`/`413` (body), `429` (pause or budget), `502`/`503`/`504` (the
engine or the cleanup model failed, is not there, or took too long).
