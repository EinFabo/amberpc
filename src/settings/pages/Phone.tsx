// Amber on the phone uses this PC as its "own server": the address and, apart, the API key to
// enter there - as with OpenAI - or both at once as a pairing link in a QR code.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DownloadEvent, KeyInfo, ModelInfo, PhoneInfo, PhoneKey } from "../../shared/types";
import type { PageProps } from "../App";
import { Progress, Row, Section, Switch } from "../ui";

export function PhonePage(props: PageProps) {
  const { s, save, toast } = props;
  const [info, setInfo] = useState<PhoneInfo | null>(null);
  const reload = useCallback(async () => setInfo(await invoke<PhoneInfo>("phone_info")), []);
  const [port, setPort] = useState(String(s.phone.port));
  useEffect(() => setPort(String(s.phone.port)), [s.phone.port]);

  // The server starts a moment after the switch; ask again until it has settled.
  useEffect(() => {
    let alive = true;
    const load = async () => {
      const i = await invoke<PhoneInfo>("phone_info");
      if (alive) setInfo(i);
    };
    void load();
    // Polled only while the API runs: its addresses and errors can change.
    const timer = s.phone.enabled ? window.setInterval(load, 1500) : undefined;
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [s.phone.enabled, s.phone.port]);

  const copy = async (text: string, what = "Address") => {
    try {
      await navigator.clipboard.writeText(text);
      toast(`${what} copied.`, true);
    } catch (e) {
      toast(`Could not copy: ${e}`);
    }
  };

  const first = info?.addresses[0];
  const firstKey = info?.keys[0];
  return (
    <>
      <h1>Phone</h1>
      <p className="lead">
        Let Amber on your Android phone use this PC to recognize your voice - with the engine set here, at home in your own network. Nothing changes in the phone app: it treats
        the PC as its own server.
      </p>
      <Section>
        <Row
          label="Let Amber on the phone use this PC"
          hint={
            s.phone.tunnel
              ? "Opens a port in your home network, and the tunnel below reaches it from the internet: whoever has an API key gets in - keep it like a password; the address alone opens nothing."
              : "Opens a port in your home network. Only devices in your own network with the address and an API key below get in."
          }
        >
          <Switch on={s.phone.enabled} onChange={(v) => save((x) => (x.phone.enabled = v))} />
        </Row>
        {s.phone.enabled && (
          <Row
            label={
              <>
                <span className={`dot ${info?.running ? "ready" : info?.error ? "failed" : "starting"}`} />
                {info?.running ? "Ready for the phone" : info?.error ? "Not running" : "Starting…"}
              </>
            }
            hint={info?.error ?? undefined}
          >
            <input
              type="text"
              style={{ width: 80 }}
              value={port}
              onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
              onBlur={() => {
                const n = Number(port);
                if (n === s.phone.port) return;
                if (n < 1024 || n > 65535) {
                  toast("Pick a port from 1024 to 65535.");
                  setPort(String(s.phone.port));
                  return;
                }
                void save((x) => (x.phone.port = n));
              }}
              title="Port"
            />
          </Row>
        )}
      </Section>

      {s.phone.enabled && info?.running && (
        <>
          <Section title="On the phone">
            {first ? (
              <div className="row" style={{ alignItems: "flex-start", gap: 24 }}>
                {info.qr && <div className="qr" dangerouslySetInnerHTML={{ __html: info.qr }} />}
                <ol className="steps">
                  <li>Phone and PC in the same network (your Wi-Fi).</li>
                  <li>
                    In Amber on the phone: <b>Engine → Own server</b>.
                  </li>
                  <li>Scan the code with the camera and paste it: Amber fills in the address and the API key. Or copy them one by one below.</li>
                  <li>Model: anything, for example whisper-1. Then tap "Test connection".</li>
                </ol>
              </div>
            ) : (
              <div className="row">
                <div className="note error">This PC has no address in a home network right now - is it connected to your router?</div>
              </div>
            )}
            {info.addresses.map((a, i) => (
              <Row key={a.url} label={i === 0 ? "Address" : `Address (${a.name})`} hint={a.url}>
                <button className="btn" onClick={() => copy(a.url)}>
                  Copy
                </button>
              </Row>
            ))}
            {first && firstKey && <ApiKeyRow secret={firstKey.secret} name={info.keys.length > 1 ? firstKey.name : undefined} copy={copy} />}
          </Section>
          <Section title="Text">
            <Row
              label="Apply the PC's cleanup"
              hint={
                s.cleanup.smart
                  ? "Fillers out and the smart cleanup, as set under Cleanup, for every phone dictation. Off when the phone does its own cleanup through this PC (below), or the text is cleaned twice."
                  : "Fillers out, as set under Cleanup. Switch on the smart cleanup there and the phone gets it too."
              }
            >
              <Switch on={s.phone.cleanup} onChange={(v) => save((x) => (x.phone.cleanup = v))} />
            </Row>
            <Row
              label="The phone's own cleanup"
              hint="Amber on the phone can clean up per app itself: choose this PC as the server for the cleanup, with any model name, e.g. “amber-cleanup”. The PC answers with its cleanup model, as set under Cleanup - so the phone's styles per app work without a model of their own."
            />
            <Row label="History and statistics" hint={
                s.history && s.statistics
                  ? "The phone's dictations are kept in the PC's history and counted in its statistics, as “phone”."
                  : s.history
                    ? "The phone's dictations are kept in the PC's history, as “phone”."
                    : s.statistics
                      ? "The history is off; the phone's dictations are only counted."
                      : "History and statistics are off: nothing of the phone's dictations is kept."
              } />
          </Section>
          {info && <TunnelSection {...props} info={info} reload={reload} copy={copy} />}
          {info && <KeysSection {...props} info={info} setInfo={setInfo} copy={copy} />}
          <p className="note">
            The phone uses the engine from the Engine page. The first time, Windows asks whether Amber may accept connections - allow it for private networks, otherwise the
            phone cannot reach the PC.
          </p>
        </>
      )}
    </>
  );
}

const TUNNEL_STATE: Record<PhoneInfo["tunnel"]["state"], [string, string]> = {
  connected: ["ready", "Connected - the phone reaches this PC from anywhere"],
  starting: ["starting", "Connecting to Cloudflare…"],
  failed: ["failed", "Not connected"],
  setup: ["starting", "Not set up yet"],
  off: ["starting", "Off"],
};

/** "https://Amber.Example.com/x" -> "amber.example.com", or "" - as the PC normalizes it. */
function hostOf(typed: string): string {
  const host = typed.trim().replace(/^[a-z]+:\/\//i, "").split(/[/?#:]/)[0].replace(/\.$/, "").toLowerCase();
  return /^[a-z0-9-]+(\.[a-z0-9-]+)+$/.test(host) ? host : "";
}

/** The phone API from anywhere: a Cloudflare Tunnel on the user's own domain. */
function TunnelSection({ s, save, toast, info, reload, copy }: PageProps & { info: PhoneInfo; reload: () => Promise<void>; copy: (text: string, what?: string) => void }) {
  const [dl, setDl] = useState<DownloadEvent | null>(null);
  const [token, setToken] = useState("");
  const [host, setHost] = useState(s.phone.publicHost);
  useEffect(() => setHost(s.phone.publicHost), [s.phone.publicHost]);

  useEffect(() => {
    const un = listen<DownloadEvent>("download", (e) => {
      if (e.payload.key !== "tunnel:cloudflared") return;
      setDl(e.payload.state === "running" ? e.payload : null);
      if (e.payload.state === "failed") toast(`Download failed: ${e.payload.message ?? ""}`);
      if (e.payload.state !== "running") void reload();
    });
    return () => void un.then((f) => f());
  }, [reload, toast]);

  const saveToken = async (value: string | null) => {
    try {
      await invoke("set_tunnel_token", { token: value });
      setToken("");
      toast(value ? "Token saved." : "Token removed.", true);
      void reload();
    } catch (e) {
      toast(String(e));
    }
  };

  const [dot, label] = TUNNEL_STATE[info.tunnel.state];
  return (
    <Section title="From anywhere">
      <Row
        label="Reach this PC from anywhere"
        hint="Through a Cloudflare Tunnel on your own domain: an HTTPS address that works on mobile data too. The access key in it keeps everyone else out."
      >
        <Switch on={s.phone.tunnel} onChange={(v) => save((x) => (x.phone.tunnel = v))} />
      </Row>
      {s.phone.tunnel && (
        <>
          <div className="row">
            <ol className="steps">
              <li>
                In Cloudflare: <b>Zero Trust → Networks → Tunnels → Create a tunnel</b>, type Cloudflared, name it e.g. “amber”.
              </li>
              <li>Copy the install command it shows for Windows - or only the token in it - and paste it below.</li>
              <li>
                Add a <b>public hostname</b>, e.g. <b>amber</b>.your-domain, service <b>HTTP</b>, URL <b>127.0.0.1:{s.phone.port}</b>. Pick a name that is hard to guess, like amber-x7k2.
              </li>
              <li>Enter that hostname below. The phone gets its address with the QR code.</li>
            </ol>
          </div>
          <Row label="cloudflared" hint="Cloudflare's tunnel program (53 MB), checked against its known fingerprint before it runs.">
            {info.tunnelDownloaded ? (
              <span style={{ color: "var(--muted)" }}>Downloaded</span>
            ) : dl ? (
              <Progress done={dl.done} total={dl.total} />
            ) : (
              <button
                className="btn primary"
                onClick={async () => {
                  setDl({ key: "tunnel:cloudflared", done: 0, total: null, state: "running" });
                  try {
                    await invoke("download", { kind: "tunnel", id: "cloudflared", select: false });
                  } catch (e) {
                    setDl(null);
                    toast(String(e));
                  }
                }}
              >
                Download
              </button>
            )}
          </Row>
          <Row label="Tunnel token" hint={info.tunnelToken ? "Saved, protected like your API keys. Paste a new one to replace it." : "From step 2. It is not shown again once saved."}>
            <input type="password" value={token} placeholder={info.tunnelToken ? "••••••••" : "eyJ…"} onChange={(e) => setToken(e.target.value)} style={{ width: 200 }} />
            <button className="btn" disabled={!token.trim()} onClick={() => saveToken(token)}>
              Save
            </button>
            {info.tunnelToken && (
              <button className="btn" onClick={() => saveToken(null)}>
                Remove
              </button>
            )}
          </Row>
          <Row label="Public hostname" hint="As set up in Cloudflare in step 3.">
            <input
              type="text"
              value={host}
              placeholder="amber.example.com"
              onChange={(e) => setHost(e.target.value)}
              onBlur={() => {
                const h = hostOf(host);
                if (host.trim() && !h) {
                  toast("That is not a hostname - e.g. amber.example.com.");
                  return;
                }
                if (h !== s.phone.publicHost) void save((x) => (x.phone.publicHost = h));
              }}
            />
          </Row>
          <Row
            label={
              <>
                <span className={`dot ${dot}`} />
                {label}
              </>
            }
            hint={info.tunnel.message ?? undefined}
          />
          {info.publicUrl && (
            <div className="row" style={{ alignItems: "flex-start", gap: 24 }}>
              {info.publicQr && <div className="qr" dangerouslySetInnerHTML={{ __html: info.publicQr }} />}
              <ol className="steps">
                <li>
                  In Amber on the phone: <b>Engine → Own server</b>.
                </li>
                <li>Scan the code and paste it, or copy the address and the API key below.</li>
                <li>It works at home and away. The cleanup through this PC (above) takes the same address.</li>
              </ol>
            </div>
          )}
          {info.publicUrl && (
            <Row label="Address from anywhere" hint={info.publicUrl}>
              <button className="btn" onClick={() => copy(info.publicUrl!)}>
                Copy
              </button>
            </Row>
          )}
          {info.publicUrl && info.keys[0] && <ApiKeyRow secret={info.keys[0].secret} copy={copy} />}
        </>
      )}
    </Section>
  );
}

/**
 * The access keys: each device or person its own - a name, the models it may use, how many
 * requests a day, its own address; one can be locked out without the others. The first is
 * the one in the address above.
 */
function KeysSection({ s, save, toast, info, setInfo, copy }: PageProps & { info: PhoneInfo; setInfo: (i: PhoneInfo) => void; copy: (text: string, what?: string) => void }) {
  const [adding, setAdding] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const keys = s.phone.keys;
  const change = (id: string, edit: (k: PhoneKey) => void) => save((x) => x.phone.keys.forEach((k) => k.id === id && edit(k)));
  const call = async (command: string, args: Record<string, unknown>, done: string) => {
    try {
      setInfo(await invoke<PhoneInfo>(command, args));
      toast(done, true);
    } catch (e) {
      toast(String(e));
    }
  };
  const add = () => {
    const name = adding.trim();
    if (!name) return;
    if (keys.some((k) => k.name.toLowerCase() === name.toLowerCase())) return toast("There is a key with that name already.");
    const id = `phone-api-${Date.now().toString(36)}`;
    save((x) => x.phone.keys.push({ id, name, models: null, perDay: null }));
    setAdding("");
    setOpen(id);
  };
  return (
    <Section title="Access keys">
      <p className="hint">
        Each key is its own way in: the same address, its own API key - give every device or person one. A key can be limited to some models and to a number of requests a day,
        and locked out without the others. The first is the one shown above.
      </p>
      {keys.map((k, i) => {
        const shown = info.keys.find((x) => x.id === k.id);
        return (
          <KeyRow
            key={k.id}
            k={k}
            shown={shown}
            first={i === 0}
            last={keys.length === 1}
            open={open === k.id}
            models={info.models}
            onOpen={() => setOpen(open === k.id ? null : k.id)}
            onChange={(edit) => change(k.id, edit)}
            taken={(name) => keys.some((x) => x.id !== k.id && x.name.toLowerCase() === name.toLowerCase())}
            toast={toast}
            onRenew={() => call("phone_new_key", { id: k.id }, `New API key for “${k.name}” - its devices need it.`)}
            onRemove={() => call("phone_remove_key", { id: k.id }, `“${k.name}” removed - its devices are locked out.`)}
            copy={copy}
          />
        );
      })}
      <div className="row">
        <input type="text" value={adding} placeholder="A new key, e.g. Laptop" onChange={(e) => setAdding(e.target.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
        <button className="btn primary" disabled={!adding.trim()} onClick={add}>
          Add key
        </button>
      </div>
    </Section>
  );
}

function KeyRow(p: {
  k: PhoneKey;
  shown?: KeyInfo;
  first: boolean;
  last: boolean;
  open: boolean;
  models: ModelInfo[];
  onOpen: () => void;
  onChange: (edit: (k: PhoneKey) => void) => void;
  onRenew: () => void;
  onRemove: () => void;
  copy: (text: string, what?: string) => void;
  taken: (name: string) => boolean;
  toast: (message: string, ok?: boolean) => void;
}) {
  const { k, shown, models } = p;
  const [name, setName] = useState(k.name);
  useEffect(() => setName(k.name), [k.name]);
  const [perDay, setPerDay] = useState(String(k.perDay ?? 2000));
  useEffect(() => setPerDay(String(k.perDay ?? 2000)), [k.perDay]);
  const rename = () => {
    const next = name.trim();
    if (!next || next === k.name) return setName(k.name);
    if (p.taken(next)) {
      p.toast("There is a key with that name already.");
      return setName(k.name);
    }
    p.onChange((x) => (x.name = next));
  };
  const allowed = (id: string) => k.models === null || k.models.includes(id);
  // All on means every model, also ones downloaded later; never none - the key would be useless.
  const toggle = (id: string) => {
    const now = k.models ?? models.map((m) => m.id);
    const next = now.includes(id) ? now.filter((m) => m !== id) : [...now, id];
    if (!next.some((m) => models.some((x) => x.id === m))) return p.toast("A key needs at least one model.");
    p.onChange((x) => (x.models = models.every((m) => next.includes(m.id)) ? null : next));
  };
  const summary = k.models === null ? "every model" : `${k.models.length} model${k.models.length === 1 ? "" : "s"}`;
  return (
    <div className="row column">
      <div className="label" style={{ cursor: "pointer" }} onClick={p.onOpen}>
        <div>
          {k.name} {p.first && <span className="hint">(the key shown above)</span>}
        </div>
        <div className="hint">
          {summary}, {k.perDay ?? 2000} requests a day {p.open ? "▴" : "▾"}
        </div>
      </div>
      {p.open && (
        <>
          <Row label="Name">
            <input type="text" value={name} onChange={(e) => setName(e.target.value)} onBlur={rename} onKeyDown={(e) => e.key === "Enter" && rename()} />
          </Row>
          {shown?.url && (
            <Row label="Address" hint={shown.url}>
              <button className="btn" onClick={() => p.copy(shown.url!)}>
                Copy
              </button>
            </Row>
          )}
          {shown?.publicUrl && (
            <Row label="Address from anywhere" hint={shown.publicUrl}>
              <button className="btn" onClick={() => p.copy(shown.publicUrl!)}>
                Copy
              </button>
            </Row>
          )}
          {shown && <ApiKeyRow secret={shown.secret} copy={p.copy} />}
          {!p.first && shown?.qr && <div className="qr" dangerouslySetInnerHTML={{ __html: shown.qr }} />}
          <div className="row column">
            <div className="label">
              <div>Models it may use</div>
              <div className="hint">The phone's choice shows only these. All on: also models downloaded later. Downloads stay on this PC.</div>
            </div>
            <div className="seg wrap">
              {models.map((m) => (
                <button key={m.id} className={allowed(m.id) ? "on" : ""} onClick={() => toggle(m.id)} title={m.note}>
                  {m.kind === "cleanup" ? "Cleanup: " : ""}
                  {m.label}
                </button>
              ))}
            </div>
          </div>
          <Row label="Requests a day" hint="Then it waits until the next day. Every key also has at most 120 in ten minutes.">
            <input
              type="number"
              min={1}
              value={perDay}
              onChange={(e) => setPerDay(e.target.value)}
              onBlur={() => {
                const v = Math.floor(Number(perDay));
                if (!Number.isFinite(v) || v < 1) return setPerDay(String(k.perDay ?? 2000));
                p.onChange((x) => (x.perDay = v === 2000 ? null : v));
              }}
            />
          </Row>
          <Row label="Renew or remove" hint="A new API key locks out the devices that have this one; the other keys go on.">
            <button className="btn danger" onClick={p.onRenew}>
              New API key
            </button>
            {!p.last && (
              <button className="btn danger" onClick={p.onRemove}>
                Remove
              </button>
            )}
          </Row>
        </>
      )}
    </div>
  );
}

/** The API key: hidden until "Show", copied on its own - an address can be shown, a key not. */
function ApiKeyRow({ secret, name, copy }: { secret: string; name?: string; copy: (text: string, what?: string) => void }) {
  const [shown, setShown] = useState(false);
  return (
    <Row label={name ? `API key (${name})` : "API key"} hint={<span className="url">{shown ? secret : "•".repeat(Math.min(secret.length, 20))}</span>}>
      <button className="btn" onClick={() => setShown(!shown)}>
        {shown ? "Hide" : "Show"}
      </button>
      <button className="btn" onClick={() => copy(secret, "API key")}>
        Copy
      </button>
    </Row>
  );
}
