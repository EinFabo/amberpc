// Small building blocks for the settings pages.

import type { ReactNode } from "react";

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
      {children}
    </div>
  );
}

export function Switch({ on, onChange }: { on: boolean; onChange: (on: boolean) => void }) {
  return <button className={`switch ${on ? "on" : ""}`} role="switch" aria-checked={on} onClick={() => onChange(!on)} />;
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
