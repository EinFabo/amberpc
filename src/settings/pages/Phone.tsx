// Amber on the phone uses this PC as its "own server": the address to enter there, as text
// and as a QR code to scan with the phone's camera.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DownloadEvent, PhoneInfo } from "../../shared/types";
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

  const copy = async (url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      toast("Address copied.", true);
    } catch (e) {
      toast(`Could not copy: ${e}`);
    }
  };

  const first = info?.addresses[0];
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
              ? "Opens a port in your home network, and the tunnel below reaches it from the internet: whoever has the address with its key gets in - keep it like a password."
              : "Opens a port in your home network. Only devices in your own network with the address below get in."
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
                  <li>
                    As the address, scan the code with the camera and paste it, or type:
                    <div className="url">{first.url}</div>
                  </li>
                  <li>Model: anything, for example whisper-1. Then tap "Test connection".</li>
                </ol>
              </div>
            ) : (
              <div className="row">
                <div className="note error">This PC has no address in a home network right now - is it connected to your router?</div>
              </div>
            )}
            {info.addresses.map((a) => (
              <Row key={a.url} label={a.name} hint={a.url}>
                <button className="btn" onClick={() => copy(a.url)}>
                  Copy
                </button>
              </Row>
            ))}
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
          {info && <TunnelSection {...props} info={info} reload={reload} />}
          <Section title="Access">
            <Row label="Access key" hint="It is part of the address. A new key locks out every phone that has the old address.">
              <button
                className="btn danger"
                onClick={async () => {
                  try {
                    setInfo(await invoke<PhoneInfo>("phone_new_key"));
                    toast("New key - enter the new address on the phone.", true);
                  } catch (e) {
                    toast(String(e));
                  }
                }}
              >
                New key
              </button>
            </Row>
          </Section>
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
function TunnelSection({ s, save, toast, info, reload }: PageProps & { info: PhoneInfo; reload: () => Promise<void> }) {
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
                <li>
                  As the address, scan the code or type:
                  <div className="url">{info.publicUrl}</div>
                </li>
                <li>It works at home and away. The cleanup through this PC (above) takes the same address.</li>
              </ol>
            </div>
          )}
        </>
      )}
    </Section>
  );
}
