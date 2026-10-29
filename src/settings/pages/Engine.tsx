import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DownloadEvent, LocalInfo, LocalItem, LocalStatus } from "../../shared/types";
import type { PageProps } from "../App";
import { Progress, Row, Section, Segmented, Switch, size } from "../ui";

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
      placeholder="e.g. Tauri, Groq, Kubernetes, Fabi"
      onChange={(e) => setText(e.target.value)}
      onBlur={() => text !== value && onSave(text)}
    />
  );
}

export function useLocal() {
  const [info, setInfo] = useState<LocalInfo | null>(null);
  const [progress, setProgress] = useState<Record<string, DownloadEvent>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});

  const reload = useCallback(async () => setInfo(await invoke<LocalInfo>("local_info")), []);

  useEffect(() => {
    void reload();
    const subs = [
      listen<DownloadEvent>("download", (e) => {
        const d = e.payload;
        setProgress((p) => ({ ...p, [d.key]: d }));
        if (d.state === "failed") setErrors((x) => ({ ...x, [d.key]: d.message ?? "failed" }));
        if (d.state !== "running") {
          setProgress((p) => {
            const { [d.key]: _, ...rest } = p;
            return rest;
          });
          void reload();
        }
      }),
      listen<LocalStatus>("local-status", (e) => setInfo((i) => (i ? { ...i, status: e.payload } : i))),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, [reload]);

  const download = async (kind: "runtime" | "model", id: string) => {
    const key = `${kind}:${id}`;
    setErrors(({ [key]: _, ...rest }) => rest);
    setProgress((p) => ({ ...p, [key]: { key, done: 0, total: null, state: "running" } }));
    await invoke("download", { kind, id });
  };

  return { info, progress, errors, reload, download };
}

function LocalEngine({ s, save, toast }: PageProps) {
  const { info, progress, errors, reload, download } = useLocal();
  if (!info) return null;

  const item = (kind: "runtime" | "model", it: LocalItem, selected: boolean, select: () => void) => {
    const key = `${kind}:${it.id}`;
    const running = progress[key] ?? (info.downloads.includes(key) ? { key, done: 0, total: null, state: "running" as const } : null);
    const recommended = (kind === "runtime" ? info.recommended[0] : info.recommended[1]) === it.id;
    const gpuMissing = kind === "runtime" && it.wantsGpu && !info.hasNvidia;
    return (
      <div className="item" key={key}>
        <div
          className={`radio ${selected ? "on" : ""} ${!it.installed ? "disabled" : ""}`}
          onClick={() => it.installed && select()}
          title={it.installed ? "" : "Download it first"}
        />
        <div className="name">
          <div>
            {it.label}
            {recommended && <span className="tag amber">recommended</span>}
            {it.installed && <span className="tag ok">installed</span>}
          </div>
          <div className="hint">
            {it.note} · {size(it.sizeMb)}
            {gpuMissing && " · no NVIDIA driver found on this PC"}
          </div>
          {errors[key] && <div className="note error">{errors[key]}</div>}
        </div>
        {running ? (
          <>
            <Progress done={running.done} total={running.total} />
            <button className="btn" onClick={() => invoke("cancel_download", { kind, id: it.id })}>
              Cancel
            </button>
          </>
        ) : it.installed ? (
          <button
            className="btn danger"
            onClick={async () => {
              try {
                await invoke("remove_download", { kind, id: it.id });
              } catch (e) {
                toast(String(e));
              }
              void reload();
            }}
          >
            Delete
          </button>
        ) : (
          <button
            className="btn"
            onClick={async () => {
              await download(kind, it.id);
              // Pick it right away, so it is used as soon as it is there.
              await select();
            }}
          >
            Download
          </button>
        )}
      </div>
    );
  };

  const st = info.status;
  return (
    <>
      <Section title="Runs on">
        {info.runtimes.map((r) => item("runtime", r, s.local.runtime === r.id, () => save((x) => (x.local.runtime = r.id))))}
      </Section>
      <Section title="Model">
        {info.models.map((m) => item("model", m, s.local.model === m.id, () => save((x) => (x.local.model = m.id))))}
        {s.local.runtime === "cpu" && info.models.find((m) => m.id === s.local.model)?.wantsGpu && (
          <div className="row">
            <div className="note">This model is slow on a CPU - expect several seconds per sentence.</div>
          </div>
        )}
      </Section>
      <Section title="Local engine">
        <Row
          label={
            <>
              <span className={`dot ${st.state}`} />
              {{ stopped: "Not loaded", starting: "Loading…", ready: "Ready", failed: "Not available" }[st.state]}
            </>
          }
          hint={st.message ?? undefined}
        >
          <button className="btn" disabled={st.state === "starting"} onClick={() => invoke("start_local")}>
            {st.state === "ready" ? "Reload" : "Load now"}
          </button>
        </Row>
        <Row label="Load when Amber starts" hint="The first dictation then does not wait for the model to load. Uses memory while Amber runs.">
          <Switch on={s.local.keepLoaded} onChange={(v) => save((x) => (x.local.keepLoaded = v))} />
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
      <Row label="Connection" hint={test ? <span className={`note ${test.ok ? "ok" : "error"}`}>{test.text}</span> : "Checks the address and the key without sending audio."}>
        <button className="btn" disabled={testing} onClick={runTest}>
          {testing ? "Testing…" : "Test connection"}
        </button>
      </Row>
    </Section>
  );
}
