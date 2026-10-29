//! Audio maths between the microphone and the engine: whatever the device delivers becomes
//! 16 kHz mono, gets checked for speech at all, and is wrapped as a WAV file.

/// What every engine gets: Whisper's native rate.
pub const TARGET_RATE: u32 = 16_000;

/// whisper.cpp refuses anything shorter than one second ("input is too short") and returns
/// nothing - a quick "Ja." would be lost. Shorter recordings are padded with silence.
pub const MIN_ENGINE_MS: u32 = 1_200;

/// Averages interleaved channels into one.
pub fn downmix(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    if channels == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Windowed-sinc resampling. Downsampling filters below the new Nyquist frequency first, so
/// a 48 kHz microphone does not fold its high end into the speech band.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    // Cutoff relative to the input's Nyquist frequency, a little below the band edge.
    let cutoff = ratio.min(1.0) * 0.94;
    let half = (16.0 / cutoff).ceil() as isize;
    let out_len = (input.len() as f64 * ratio).floor() as usize;
    let last = input.len() as isize - 1;

    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let t = n as f64 / ratio;
        let center = t.floor() as isize;
        let mut acc = 0.0f64;
        let mut weights = 0.0f64;
        for k in (center - half + 1)..=(center + half) {
            if k < 0 || k > last {
                continue;
            }
            let x = t - k as f64;
            let w = cutoff * sinc(cutoff * x) * blackman(x / half as f64);
            acc += input[k as usize] as f64 * w;
            weights += w;
        }
        // Normalizing by the weights keeps DC gain at 1, also at the edges.
        out.push(if weights.abs() > 1e-12 { (acc / weights) as f32 } else { 0.0 });
    }
    out
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// Blackman window over -1..1.
fn blackman(x: f64) -> f64 {
    if x.abs() >= 1.0 {
        return 0.0;
    }
    let a = std::f64::consts::PI * (x + 1.0);
    0.42 - 0.5 * a.cos() + 0.08 * (2.0 * a).cos()
}

/// Root mean square of a block.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| s as f64 * s as f64).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// A level for the bubble and the meter: -60 dBFS and below is 0, -10 dBFS and above is 1.
pub fn level(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db + 60.0) / 50.0).clamp(0.0, 1.0)
}

/// Frames of 30 ms; at least 120 ms of them must stand clearly above the quietest frame.
/// Pure silence goes to no engine - Whisper invents subtitles on it.
///
/// The floor is the quietest frame, capped, so a recording that is speech from the first
/// millisecond (push to talk) still has a usable floor instead of its own voice.
pub fn has_speech(samples: &[f32], rate: u32) -> bool {
    let frame = (rate as usize * 30 / 1000).max(1);
    let levels: Vec<f32> = samples.chunks(frame).filter(|c| c.len() == frame).map(rms).collect();
    if levels.is_empty() {
        return false;
    }
    let floor = levels.iter().cloned().fold(f32::INFINITY, f32::min).min(0.02);
    let threshold = (floor * 4.0).clamp(0.006, 0.03);
    levels.iter().filter(|&&l| l > threshold).count() >= 4
}

/// Pads with silence up to [MIN_ENGINE_MS], a little before and the rest after.
pub fn pad_for_engine(samples: &[f32], rate: u32) -> Vec<f32> {
    let min = (rate as u64 * MIN_ENGINE_MS as u64 / 1000) as usize;
    if samples.len() >= min {
        return samples.to_vec();
    }
    let lead = (rate as usize / 10).min(min - samples.len());
    let mut out = vec![0.0; lead];
    out.extend_from_slice(samples);
    out.resize(min, 0.0);
    out
}

pub fn duration_ms(samples: usize, rate: u32) -> u64 {
    if rate == 0 {
        0
    } else {
        samples as u64 * 1000 / rate as u64
    }
}

/// 16-bit PCM WAV, mono.
pub fn encode_wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Everything from the device to the engine in one go.
pub struct Prepared {
    pub wav: Vec<u8>,
    /// Length of what was actually spoken, before padding.
    pub spoken_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    Silence,
}

pub fn prepare(mono: &[f32], rate: u32) -> Result<Prepared, Rejected> {
    let resampled = resample(mono, rate, TARGET_RATE);
    if !has_speech(&resampled, TARGET_RATE) {
        return Err(Rejected::Silence);
    }
    let spoken_ms = duration_ms(resampled.len(), TARGET_RATE);
    let padded = pad_for_engine(&resampled, TARGET_RATE);
    Ok(Prepared { wav: encode_wav(&padded, TARGET_RATE), spoken_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, ms: u32, amp: f32) -> Vec<f32> {
        let n = rate as usize * ms as usize / 1000;
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(downmix(&[0.3, 0.1], 1), vec![0.3, 0.1]);
    }

    #[test]
    fn resample_keeps_length_dc_and_speech_band() {
        let dc = vec![0.25f32; 48_000];
        let out = resample(&dc, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-3));

        let tone = resample(&sine(1_000.0, 48_000, 500, 0.5), 48_000, 16_000);
        let middle = &tone[1_000..7_000];
        assert!((rms(middle) - 0.5 / 2f32.sqrt()).abs() < 0.01, "1 kHz passes: {}", rms(middle));
    }

    #[test]
    fn resample_removes_what_would_alias() {
        // 12 kHz cannot exist at 16 kHz; it must not fold down to 4 kHz.
        let out = resample(&sine(12_000.0, 48_000, 500, 0.5), 48_000, 16_000);
        assert!(rms(&out[1_000..7_000]) < 0.01, "alias leaked: {}", rms(&out[1_000..7_000]));
    }

    #[test]
    fn resample_upsamples_too() {
        let out = resample(&sine(440.0, 8_000, 250, 0.5), 8_000, 16_000);
        assert_eq!(out.len(), 4_000);
        assert!((rms(&out[500..3_500]) - 0.354).abs() < 0.01);
    }

    #[test]
    fn level_maps_dbfs() {
        assert_eq!(level(0.0), 0.0);
        assert_eq!(level(0.001), 0.0); // -60 dBFS
        assert!((level(0.0316) - 0.6).abs() < 0.01); // -30 dBFS
        assert_eq!(level(1.0), 1.0);
    }

    #[test]
    fn silence_and_hiss_are_not_speech() {
        assert!(!has_speech(&vec![0.0; 16_000], 16_000));
        let hiss: Vec<f32> = (0..16_000).map(|i| if i % 2 == 0 { 0.002 } else { -0.002 }).collect();
        assert!(!has_speech(&hiss, 16_000));
    }

    #[test]
    fn a_short_word_counts_even_when_it_starts_at_once() {
        // 150 ms of voice right at the start (push to talk), then quiet.
        let mut s = sine(220.0, 16_000, 150, 0.2);
        s.extend(vec![0.0005; 8_000]);
        assert!(has_speech(&s, 16_000));
        // Voice from the first to the last frame.
        assert!(has_speech(&sine(220.0, 16_000, 400, 0.1), 16_000));
    }

    #[test]
    fn short_audio_is_padded_to_the_engine_minimum() {
        let padded = pad_for_engine(&vec![0.5; 4_000], 16_000);
        assert_eq!(padded.len(), 19_200);
        assert_eq!(padded[0], 0.0);
        assert_eq!(padded[1_600], 0.5);
        assert_eq!(pad_for_engine(&vec![0.1; 20_000], 16_000).len(), 20_000);
    }

    #[test]
    fn wav_header_is_valid() {
        let wav = encode_wav(&[0.0, 1.0, -1.0], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), i16::MAX);
        assert_eq!(i16::from_le_bytes([wav[48], wav[49]]), -i16::MAX);
    }

    #[test]
    fn prepare_rejects_silence_and_wraps_speech() {
        assert_eq!(prepare(&vec![0.0; 48_000], 48_000).err(), Some(Rejected::Silence));
        let p = prepare(&sine(300.0, 48_000, 600, 0.2), 48_000).unwrap();
        assert_eq!(p.spoken_ms, 600);
        assert_eq!(p.wav.len(), 44 + 19_200 * 2);
    }
}
