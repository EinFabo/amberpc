import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PageProps } from "../App";
import { Row, Section, Segmented, Switch } from "../ui";
import { DownloadItem, StatusLabel, useLocal } from "../downloads";

const LANGUAGES: [string, string][] = [
  ["auto", "Detect automatically"],
  ["de", "German"],
  ["en", "English"],
  ["fr", "French"],
  ["es", "Spanish"],
  ["it", "Italian"],
  ["nl", "Dutch"],
  ["pl", "Polish"],
  ["tr", "Turkish"],
];

export function EnginePage(p: PageProps) {
  const { s, save } = p;
  return (
    <>
      <h1>Engine</h1>
      <p className="lead">Where your voice is turned into text. Locally nothing leaves this PC; a server is quick on any PC but needs an API key.</p>
      <div className="cards">
        <button className={`card ${s.engine === "local" ? "on" : ""}`} onClick={() => save((x) => (x.engine = "local"))}>
          <div className="title">On this PC</div>
          <div className="hint">whisper.cpp on your CPU or NVIDIA GPU. Private, works offline, free.</div>
        </button>
        <button className={`card ${s.engine === "server" ? "on" : ""}`} onClick={() => save((x) => (x.engine = "server"))}>
          <div className="title">Server with API key</div>
          <div className="hint">Groq, OpenAI or your own server. Your recording is sent there.</div>
        </button>
      </div>
      {s.engine === "local" ? <LocalEngine {...p} /> : <ServerEngine {...p} />}
      <Section title="Language">
        <Row label="Spoken language" hint="A fixed language makes Whisper translate everything else into it - keep it on automatic if you switch languages.">
          <select value={s.language} onChange={(e) => save((x) => (x.language = e.target.value))}>
            {LANGUAGES.map(([v, l]) => (
              <option key={v} value={v}>
                {l}
              </option>
            ))}
          </select>
        </Row>
        <div className="row column">
          <div className="label">
            <div>Vocabulary</div>
            <div className="hint">Names and terms the engine should expect, separated by commas.</div>
          </div>
          <Vocabulary value={s.vocabulary} onSave={(v) => save((x) => (x.vocabulary = v))} />
        </div>
      </Section>
    </>
  );
}

function Vocabulary({ value, onSave }: { value: string; onSave: (v: string) => void }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  return (
    <textarea
      value={text}
      placeholder="e.g. Tauri, Groq, Kubernetes, Anna"
      onChange={(e) => setText(e.target.value)}
      onBlur={() => text !== value && onSave(text)}
    />
  );
}

function LocalEngine({ s, save, toast }: PageProps) {
  // A download is selected by Amber once it is complete, so the engine that works now keeps
  // working until then.
  const local = useLocal();
  const info = local.info;
  if (!info) return null;

  const st = info.status;
  return (
    <>
      <Section title="Runs on">
        {info.runtimes.map((r) => (
          <DownloadItem
            key={r.id}
            local={local}
            kind="runtime"
            item={r}
            selected={s.local.runtime === r.id}
            onSelect={() => save((x) => (x.local.runtime = r.id))}
            recommended={info.recommended[0] === r.id}
            warning={r.wantsGpu && !info.hasNvidia ? "no NVIDIA driver found on this PC" : undefined}
            toast={toast}
          />
        ))}
      </Section>
      <Section title="Model">
        {info.models
          // A replaced model shows only while it is still on disk - or still selected.
          .filter((m) => !m.replaced || m.installed || m.id === s.local.model)
          .map((m) => (
            <DownloadItem
              key={m.id}
              local={local}
              kind="model"
              item={m}
              selected={s.local.model === m.id}
              onSelect={() => save((x) => (x.local.model = m.id))}
              recommended={info.recommended[1] === m.id}
              measured={info.measured[m.id]}
              toast={toast}
            />
          ))}
        {info.models.find((m) => m.id === s.local.model)?.replaced && (
          <div className="row">
            <div className="note">
              Your model is an older one that always works through 30 seconds. The tuned models above stop where your speech stops - as accurate and several times faster.
            </div>
          </div>
        )}
        {s.local.runtime === "cpu" && info.models.find((m) => m.id === s.local.model)?.wantsGpu && (
          <div className="row">
            <div className="note">This model is slow on a CPU - expect several seconds per sentence.</div>
          </div>
        )}
      </Section>
      <Section title="Local engine">
        <Row label={<StatusLabel status={st} />} hint={st.message ?? undefined}>
          {st.state !== "ready" && (
            <button className="btn" disabled={st.state === "starting"} onClick={() => invoke("start_local")}>
              Load now
            </button>
          )}
        </Row>
        <Row label="Load when Amber starts" hint="The first dictation then does not wait for the model to load. Uses memory while Amber runs.">
          <Switch on={s.local.keepLoaded} onChange={(v) => save((x) => (x.local.keepLoaded = v))} />
        </Row>
        <Row
          label="Recognize while speaking"
          hint="Long dictations are cut at pauses and recognized part by part while you keep talking - after you stop, only the last part is left to wait for."
        >
          <Switch on={s.local.chunking} onChange={(v) => save((x) => (x.local.chunking = v))} />
        </Row>
        <Row label="CPU threads" hint="Automatic uses half of your processor's threads.">
          <select value={s.local.threads} onChange={(e) => save((x) => (x.local.threads = Number(e.target.value)))}>
            <option value={0}>Automatic</option>
            {[2, 4, 6, 8, 10, 12, 16].map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </Row>
      </Section>
    </>
  );
}

function ServerEngine({ s, save, ov, refresh, toast }: PageProps) {
  const provider = ov.providers.find((p) => p.id === s.server.provider) ?? ov.providers[0];
  const [key, setKey] = useState("");
  const [baseUrl, setBaseUrl] = useState(s.server.baseUrl);
  const [test, setTest] = useState<{ ok: boolean; text: string } | null>(null);
  const [testing, setTesting] = useState(false);
  useEffect(() => setBaseUrl(s.server.baseUrl), [s.server.baseUrl]);
  useEffect(() => setTest(null), [provider.id]);

  const custom = provider.id === "custom";

  const saveKey = async (value: string | null) => {
    try {
      await invoke("set_api_key", { provider: provider.id, key: value });
      setKey("");
      await refresh();
      toast(value ? "Key saved." : "Key removed.", true);
    } catch (e) {
      toast(String(e));
    }
  };

  const runTest = async () => {
    setTesting(true);
    setTest(null);
    try {
      const ms = await invoke<number>("test_server", { provider: provider.id, baseUrl, key: key || null });
      setTest({ ok: true, text: `Connected - ${ms} ms.${key ? " Save the key to use it." : ""}` });
    } catch (e) {
      setTest({ ok: false, text: String(e) });
    }
    setTesting(false);
  };

  return (
    <Section title="Server">
      <Row label="Provider">
        <Segmented
          value={provider.id}
          options={ov.providers.map((p) => [p.id, p.name] as [string, string])}
          onChange={(id) =>
            save((x) => {
              x.server.provider = id;
              x.server.model = ov.providers.find((p) => p.id === id)?.models[0]?.id ?? x.server.model;
            })
          }
        />
      </Row>
      {custom && (
        <Row label="Address" hint="The OpenAI-compatible base URL, e.g. http://192.168.1.20:8000/v1. Plain http only inside your own network.">
          <input
            type="text"
            style={{ width: 300 }}
            value={baseUrl}
            placeholder="https://…/v1"
            onChange={(e) => setBaseUrl(e.target.value)}
            onBlur={() => baseUrl !== s.server.baseUrl && save((x) => (x.server.baseUrl = baseUrl.trim()))}
          />
        </Row>
      )}
      <Row label="Model" hint={provider.models.find((m) => m.id === s.server.model)?.note}>
        {provider.models.length > 0 ? (
          <select value={s.server.model} onChange={(e) => save((x) => (x.server.model = e.target.value))}>
            {provider.models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
          </select>
        ) : (
          <input
            type="text"
            defaultValue={s.server.model}
            placeholder="whisper-1"
            onBlur={(e) => e.target.value !== s.server.model && save((x) => (x.server.model = e.target.value.trim()))}
          />
        )}
      </Row>
      <Row
        label="API key"
        hint={
          <>
            {provider.hasKey ? "A key is saved (encrypted for your Windows account)." : provider.needsKey ? "No key saved yet." : "Optional for your own server."}{" "}
            {provider.keyUrl && (
              <a className="link" onClick={() => invoke("open_url", { url: provider.keyUrl })}>
                Get a key
              </a>
            )}
          </>
        }
      >
        <input type="password" style={{ width: 220 }} value={key} placeholder={provider.hasKey ? "••••••••" : "Paste key"} onChange={(e) => setKey(e.target.value)} />
        <button className="btn primary" disabled={!key.trim()} onClick={() => saveKey(key)}>
          Save
        </button>
        {provider.hasKey && (
          <button className="btn danger" onClick={() => saveKey(null)}>
            Remove
          </button>
        )}
      </Row>
      <Row
        label="Fall back to this PC"
        hint="When the server cannot be reached (offline, down), the recording is recognized by the model set under “On this PC” instead of being lost - if it is downloaded. Never the other way round."
      >
        <Switch on={s.server.localFallback} onChange={(v) => save((x) => (x.server.localFallback = v))} />
      </Row>
      <Row label="Connection" hint={test ? <span className={`note ${test.ok ? "ok" : "error"}`}>{test.text}</span> : "Checks the address and the key without sending audio."}>
        <button className="btn" disabled={testing} onClick={runTest}>
          {testing ? "Testing…" : "Test connection"}
        </button>
      </Row>
    </Section>
  );
}
