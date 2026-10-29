// Amber on the phone uses this PC as its "own server": the address to enter there, as text
// and as a QR code to scan with the phone's camera.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PhoneInfo } from "../../shared/types";
import type { PageProps } from "../App";
import { Row, Section, Switch } from "../ui";

export function PhonePage({ s, save, toast }: PageProps) {
  const [info, setInfo] = useState<PhoneInfo | null>(null);
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
    const timer = window.setInterval(load, 1500);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [s.phone.enabled, s.phone.port]);

  const copy = async (url: string) => {
    await navigator.clipboard.writeText(url);
    toast("Address copied.", true);
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
        <Row label="Let Amber on the phone use this PC" hint="Opens a port in your home network. Only devices in your own network with the address below get in.">
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
              onBlur={() => Number(port) !== s.phone.port && save((x) => (x.phone.port = Number(port) || 8765))}
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
