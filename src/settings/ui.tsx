// Small building blocks for the settings pages.

import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";

/** A row's label, for the switch in it - screen readers announce what it switches. */
const RowLabel = createContext<string | undefined>(undefined);

export function Section({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section>
      {title && <h2>{title}</h2>}
      {children}
    </section>
  );
}

export function Row({ label, hint, children }: { label: ReactNode; hint?: ReactNode; children?: ReactNode }) {
  return (
    <div className="row">
      <div className="label">
        <div>{label}</div>
        {hint && <div className="hint">{hint}</div>}
      </div>
      <RowLabel.Provider value={typeof label === "string" ? label : undefined}>{children}</RowLabel.Provider>
    </div>
  );
}

export function Switch({ on, onChange }: { on: boolean; onChange: (on: boolean) => void }) {
  const label = useContext(RowLabel);
  return <button className={`switch ${on ? "on" : ""}`} role="switch" aria-checked={on} aria-label={label} onClick={() => onChange(!on)} />;
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: [T, string][];
  onChange: (v: T) => void;
}) {
  return (
    <div className="seg">
      {options.map(([v, label]) => (
        <button key={v} className={v === value ? "on" : ""} onClick={() => onChange(v)}>
          {label}
        </button>
      ))}
    </div>
  );
}

export function Progress({ done, total }: { done: number; total?: number | null }) {
  const pct = total ? Math.min(100, (done / total) * 100) : 0;
  return (
    <div className="progress" title={total ? `${Math.round(pct)} %` : `${mb(done)} MB`}>
      <div style={{ width: `${pct}%` }} />
    </div>
  );
}

export function mb(bytes: number) {
  return Math.round(bytes / 1048576);
}

export function size(mbs: number) {
  return mbs >= 1000 ? `${(mbs / 1024).toFixed(1)} GB` : `${mbs} MB`;
}

/** A range input that follows the hand at once but saves only once it rests. */
export function Slider({ value, min, max, step = 1, onCommit }: { value: number; min: number; max: number; step?: number; onCommit: (v: number) => void }) {
  const [local, setLocal] = useState(value);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => setLocal(value), [value]);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return (
    <input
      type="range"
      min={min}
      max={max}
      step={step}
      value={local}
      onChange={(e) => {
        const v = Number(e.target.value);
        setLocal(v);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(() => onCommit(v), 200);
      }}
    />
  );
}
