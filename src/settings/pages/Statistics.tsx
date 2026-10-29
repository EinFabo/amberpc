// Counts and timings per dictation - never the text - kept on this PC.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { StatsSummary } from "../../shared/types";
import type { PageProps } from "../App";
import { Row, Section, Switch } from "../ui";

export function StatisticsPage({ s, save }: PageProps) {
  const [sum, setSum] = useState<StatsSummary | null>(null);
  const load = async () => setSum(await invoke<StatsSummary>("stats_summary"));
  useEffect(() => {
    void load();
  }, []);

  const max = Math.max(1, ...(sum?.lastDays ?? [1]));
  return (
    <>
      <h1>Statistics</h1>
      <p className="lead">Counted on this PC only, from word counts and timings - never what you said. Time saved compares typing at 40 words a minute with speaking plus waiting.</p>
      <Section>
        <Row label="Count dictations">
          <Switch on={s.statistics} onChange={(v) => save((x) => (x.statistics = v))} />
        </Row>
      </Section>
      {sum && sum.totalDictations > 0 && (
        <>
          <div className="stats">
            <Tile value={sum.wordsToday} label={`words today · ${sum.dictationsToday} dictations`} />
            <Tile value={`${Math.round(sum.minutesSavedToday)} min`} label="saved today" />
            <Tile value={sum.streakDays} label={sum.streakDays === 1 ? "day in a row" : "days in a row"} />
            <Tile value={`${Math.round(sum.minutesSavedTotal)} min`} label={`saved over ${sum.totalWords} words`} />
          </div>
          <Section title="Words, last 14 days">
            <div className="bars" aria-label="Words per day">
              {sum.lastDays.map((w, i) => (
                <div key={i} className="bar" title={`${w} words`}>
                  <div style={{ height: `${(w / max) * 100}%` }} />
                </div>
              ))}
            </div>
          </Section>
          {sum.wordsByProgram.length > 0 && (
            <Section title="Programs">
              {sum.wordsByProgram.slice(0, 8).map(([program, words]) => (
                <Row key={program} label={program}>
                  <span className="note">{words} words</span>
                </Row>
              ))}
            </Section>
          )}
          <Section title="Wait after speaking (median)">
            {Object.entries(sum.medianWaitMs).map(([engine, ms]) => (
              <Row key={engine} label={engine}>
                <span className="note">{(ms / 1000).toFixed(1)} s</span>
              </Row>
            ))}
            {sum.speakingWpm !== null && (
              <Row label="Your speaking pace">
                <span className="note">{sum.speakingWpm} words a minute</span>
              </Row>
            )}
          </Section>
          <div className="row">
            <button
              className="btn danger"
              onClick={async () => {
                await invoke("stats_clear");
                void load();
              }}
            >
              Clear statistics
            </button>
          </div>
        </>
      )}
      {sum && sum.totalDictations === 0 && <p className="note">Nothing counted yet - the numbers fill after a few dictations.</p>}
    </>
  );
}

function Tile({ value, label }: { value: string | number; label: string }) {
  return (
    <div className="tile">
      <div className="value">{value}</div>
      <div className="note">{label}</div>
    </div>
  );
}
