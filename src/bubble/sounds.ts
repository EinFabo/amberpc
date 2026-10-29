// Short, quiet cues so a dictation can be started and stopped without looking.

let audio: AudioContext | null = null;

const CUES: Record<"start" | "stop" | "error", [number, number][]> = {
  // [frequency, start offset in s]
  start: [[660, 0], [880, 0.07]],
  stop: [[880, 0], [660, 0.07]],
  error: [[330, 0], [247, 0.1]],
};

export function chime(kind: keyof typeof CUES) {
  try {
    audio ??= new AudioContext();
    const t0 = audio.currentTime + 0.01;
    for (const [freq, offset] of CUES[kind]) {
      const osc = audio.createOscillator();
      const gain = audio.createGain();
      osc.type = "sine";
      osc.frequency.value = freq;
      const t = t0 + offset;
      gain.gain.setValueAtTime(0, t);
      gain.gain.linearRampToValueAtTime(0.08, t + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.12);
      osc.connect(gain).connect(audio.destination);
      osc.start(t);
      osc.stop(t + 0.13);
    }
  } catch {
    // No audio device - the bubble still shows the state.
  }
}
