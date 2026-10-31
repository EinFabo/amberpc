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
///
/// Polyphase: for a rational ratio L/M (48 kHz -> 16 kHz is 1/3, 44.1 kHz is 160/441) every
/// output sample falls on one of L fractional positions, so the filter taps are computed once
/// per position instead of once per sample.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    let g = gcd(from as u64, to as u64);
    let (l, m) = (to as u64 / g, from as u64 / g);
    let ratio = to as f64 / from as f64;
    // Cutoff relative to the input's Nyquist frequency, a little below the band edge.
    let cutoff = ratio.min(1.0) * 0.94;
    let half = (16.0 / cutoff).ceil() as i64;
    let taps = (2 * half) as usize;
    let out_len = (input.len() as f64 * ratio).floor() as usize;
    let last = input.len() as i64 - 1;

    let weight = |x: f64| cutoff * sinc(cutoff * x) * blackman(x / half as f64);
    // Odd device rates would need an enormous table; those few compute the taps directly.
    let table: Option<Vec<f32>> = (l <= 4096).then(|| {
        let mut t = Vec::with_capacity(l as usize * taps);
        for phase in 0..l {
            let frac = phase as f64 / l as f64;
            for j in 0..taps as i64 {
                t.push(weight(frac + (half - 1 - j) as f64) as f32);
            }
        }
        t
    });

    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len as u64 {
        let pos = n * m;
        let center = (pos / l) as i64;
        let phase = pos % l;
        let first = center - half + 1;
        let mut acc = 0.0f64;
        let mut weights = 0.0f64;
        match &table {
            Some(t) => {
                let row = &t[phase as usize * taps..(phase as usize + 1) * taps];
                if first >= 0 && first + taps as i64 - 1 <= last {
                    let window = &input[first as usize..first as usize + taps];
                    for (x, w) in window.iter().zip(row) {
                        acc += (*x * *w) as f64;
                        weights += *w as f64;
                    }
                } else {
                    for (j, w) in row.iter().enumerate() {
                        let k = first + j as i64;
                        if (0..=last).contains(&k) {
                            acc += (input[k as usize] * *w) as f64;
                            weights += *w as f64;
                        }
                    }
                }
            }
            None => {
                let t = pos as f64 / l as f64;
                for k in first..=(center + half) {
                    if (0..=last).contains(&k) {
                        let w = weight(t - k as f64);
                        acc += input[k as usize] as f64 * w;
                        weights += w;
                    }
                }
            }
        }
        // Normalizing by the weights keeps DC gain at 1, also at the edges.
        out.push(if weights.abs() > 1e-12 { (acc / weights) as f32 } else { 0.0 });
    }
    out
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
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

/// An upload to the phone API as mono samples and their rate: WAV, or Ogg Opus (a phone on
/// mobile data sends a tenth of the bytes). Told apart by the first bytes, not the filename.
pub fn decode_upload(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    match bytes.get(..4) {
        Some(b"RIFF") => decode_wav(bytes),
        Some(b"OggS") => decode_ogg_opus(bytes).map(|s| (s, TARGET_RATE)),
        _ => Err("Amber reads WAV or Ogg Opus".into()),
    }
}

/// Ogg Opus (RFC 7845), decoded straight to 16 kHz mono; the encoder's lead-in (pre-skip) is
/// dropped and its output gain applied.
fn decode_ogg_opus(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let broken = |what: &str| format!("broken Ogg Opus: {what}");
    let mut reader = ogg::PacketReader::new(std::io::Cursor::new(bytes));
    let mut next = || reader.read_packet().map_err(|e| broken(&e.to_string()));
    let head = next()?.ok_or_else(|| broken("empty"))?.data;
    if head.len() < 19 || &head[..8] != b"OpusHead" {
        return Err("Ogg, but not Opus - Amber reads WAV or Ogg Opus".into());
    }
    let (channels, mapping) = (head[9], head[18]);
    if !(1..=2).contains(&channels) || mapping != 0 {
        return Err(broken("only mono or stereo"));
    }
    // Pre-skip counts 48 kHz samples.
    let skip = u16::from_le_bytes([head[10], head[11]]) as usize * TARGET_RATE as usize / 48_000;
    let gain = 10f32.powf(i16::from_le_bytes([head[16], head[17]]) as f32 / (20.0 * 256.0));
    next()?.filter(|tags| tags.data.starts_with(b"OpusTags")).ok_or_else(|| broken("no OpusTags"))?;

    // Decoding to one channel mixes a stereo stream down.
    let mut decoder = opus_decoder::OpusDecoder::new(TARGET_RATE, 1).map_err(|e| broken(&e.to_string()))?;
    let max = (MAX_DECODE_SECONDS * TARGET_RATE as u64) as usize;
    let mut frame = vec![0f32; decoder.max_frame_size_per_channel()];
    let mut samples = Vec::new();
    while let Some(packet) = next()? {
        let n = decoder.decode_float(&packet.data, &mut frame, false).map_err(|e| broken(&e.to_string()))?;
        samples.extend(frame[..n].iter().map(|s| s * gain));
        if samples.len() > max + skip {
            return Err(format!("longer than {} minutes", MAX_DECODE_SECONDS / 60));
        }
    }
    if samples.len() <= skip {
        return Err(broken("no audio"));
    }
    samples.drain(..skip);
    Ok(samples)
}

/// The longest recording [decode_wav] accepts.
pub const MAX_DECODE_SECONDS: u64 = 15 * 60;

/// Reads a WAV file as mono samples and its rate: 16-bit PCM or 32-bit float, any channel
/// count, other chunks (LIST, fact, ...) skipped. What other apps send to the phone API.
pub fn decode_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let mut pos = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(pos + 4) as usize;
        let body = pos + 8;
        let end = body.saturating_add(len).min(bytes.len());
        match id {
            b"fmt " if len >= 16 && body + 16 <= bytes.len() => {
                format = Some((u16_at(body), u16_at(body + 2), u32_at(body + 4), u16_at(body + 14)));
            }
            b"data" => {
                let (tag, channels, rate, bits) = format.ok_or("WAV without a format chunk")?;
                let data = &bytes[body..end];
                let interleaved: Vec<f32> = match (tag, bits) {
                    (1, 16) => data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect(),
                    (3, 32) => data.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
                    _ => return Err(format!("unsupported WAV encoding (format {tag}, {bits} bit) - send 16-bit PCM")),
                };
                // Rates and channel counts real recorders use. A made-up rate would make the
                // resampler allocate without bound (1 Hz -> 16 000 times the input).
                if !(8_000..=192_000).contains(&rate) || !(1..=8).contains(&channels) {
                    return Err(format!("unsupported WAV ({rate} Hz, {channels} channels)"));
                }
                let frames = interleaved.len() / channels as usize;
                if frames as u64 > rate as u64 * MAX_DECODE_SECONDS {
                    return Err("WAV longer than 15 minutes".into());
                }
                return Ok((downmix(&interleaved, channels), rate));
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        pos = body.saturating_add(len + (len & 1));
    }
    Err("WAV without audio data".into())
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

    const TONE_OGG: &[u8] = include_bytes!("../tests/data/tone-440-880.ogg");

    /// Zero crossings per second, half of which is the frequency of a pure tone.
    fn frequency(samples: &[f32], rate: u32) -> f32 {
        let crossings = samples.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        crossings as f32 / 2.0 / (samples.len() as f32 / rate as f32)
    }

    #[test]
    fn ogg_opus_from_the_phone_is_read() {
        // One second made with libopus: 440 Hz, then 880 Hz; 3 KB instead of 31 KB as WAV.
        let (samples, rate) = decode_upload(TONE_OGG).unwrap();
        assert_eq!(rate, 16_000);
        assert!((15_000..=16_100).contains(&samples.len()), "{} samples", samples.len());
        let (first, second) = samples.split_at(samples.len() / 2);
        let (f1, f2) = (frequency(&first[1_000..7_000], rate), frequency(&second[1_000..7_000], rate));
        assert!((420.0..460.0).contains(&f1), "{f1} Hz");
        assert!((850.0..910.0).contains(&f2), "{f2} Hz");
        assert!(rms(&samples) > 0.1, "loud, as encoded");
    }

    #[test]
    fn uploads_are_told_apart_by_their_first_bytes() {
        let wav = encode_wav(&[0.1; 1_600], 16_000);
        assert_eq!(decode_upload(&wav).unwrap().0.len(), 1_600);
        assert!(decode_upload(b"ID3\x04 an mp3").unwrap_err().contains("WAV or Ogg Opus"));
        assert!(decode_upload(b"").is_err());
    }

    #[test]
    fn broken_ogg_is_refused_not_a_crash() {
        // Cut anywhere, or bytes flipped: an error, never a panic or a hang.
        for cut in [4, 27, 60, 200, TONE_OGG.len() / 2, TONE_OGG.len() - 1] {
            let _ = decode_upload(&TONE_OGG[..cut]);
        }
        assert!(decode_upload(&TONE_OGG[..40]).is_err(), "no audio after the head");
        let mut flipped = TONE_OGG.to_vec();
        for i in (100..flipped.len()).step_by(37) {
            flipped[i] ^= 0x5a;
        }
        let _ = decode_upload(&flipped);
        // An Ogg file that is not Opus (Vorbis, say).
        let mut vorbis = TONE_OGG.to_vec();
        let at = vorbis.windows(8).position(|w| w == b"OpusHead").unwrap();
        vorbis[at..at + 8].copy_from_slice(b"\x01vorbis\0");
        assert!(decode_upload(&vorbis).unwrap_err().contains("Opus"));
    }

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
    fn resample_odd_rates() {
        // 44.1 kHz goes through a 160-phase table; 44 057 Hz (coprime with 16 kHz, 16 000
        // phases) through the direct path.
        for from in [44_100, 44_057, 22_050] {
            let out = resample(&sine(1_000.0, from, 500, 0.5), from, 16_000);
            assert!(out.len().abs_diff(8_000) <= 1, "{from}: {}", out.len());
            assert!((rms(&out[1_000..7_000]) - 0.354).abs() < 0.01, "{from}: {}", rms(&out[1_000..7_000]));
        }
    }

    #[test]
    fn resample_is_fast_enough() {
        // Two minutes at 48 kHz must not keep the user waiting (debug build: generous bound).
        let input = sine(300.0, 48_000, 120_000, 0.3);
        let t0 = std::time::Instant::now();
        let out = resample(&input, 48_000, 16_000);
        assert_eq!(out.len(), 1_920_000);
        assert!(t0.elapsed().as_secs_f64() < 8.0, "{:?}", t0.elapsed());
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
    fn decode_reads_what_encode_wrote() {
        let wav = encode_wav(&[0.0, 0.5, -0.5], 16_000);
        let (samples, rate) = decode_wav(&wav).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(samples.len(), 3);
        assert!((samples[1] - 0.5).abs() < 1e-3 && (samples[2] + 0.5).abs() < 1e-3);
    }

    #[test]
    fn decode_handles_stereo_float_and_extra_chunks() {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF\0\0\0\0WAVE");
        wav.extend_from_slice(b"LIST");
        wav.extend_from_slice(&3u32.to_le_bytes());
        wav.extend_from_slice(b"abc\0"); // odd length, padded
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&3u16.to_le_bytes()); // float
        wav.extend_from_slice(&2u16.to_le_bytes()); // stereo
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&(48_000u32 * 8).to_le_bytes());
        wav.extend_from_slice(&8u16.to_le_bytes());
        wav.extend_from_slice(&32u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&16u32.to_le_bytes());
        for v in [1.0f32, 0.0, 0.5, 0.5] {
            wav.extend_from_slice(&v.to_le_bytes());
        }
        let (samples, rate) = decode_wav(&wav).unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(samples, vec![0.5, 0.5]);
    }

    #[test]
    fn decode_refuses_what_it_cannot_read() {
        assert!(decode_wav(b"ID3 not a wav").is_err());
        let mut wav = encode_wav(&[0.1; 4], 16_000);
        wav[34] = 24; // 24-bit
        assert!(decode_wav(&wav).unwrap_err().contains("16-bit"));
        for (rate, channels) in [(1u32, 1u16), (4_000_000_004, 1), (16_000, 0), (16_000, 9)] {
            let mut wav = encode_wav(&[0.1; 4], 16_000);
            wav[24..28].copy_from_slice(&rate.to_le_bytes());
            wav[22..24].copy_from_slice(&channels.to_le_bytes());
            assert!(decode_wav(&wav).unwrap_err().contains("unsupported"), "{rate} Hz, {channels} ch");
        }
    }

    #[test]
    fn prepare_rejects_silence_and_wraps_speech() {
        assert_eq!(prepare(&vec![0.0; 48_000], 48_000).err(), Some(Rejected::Silence));
        let p = prepare(&sine(300.0, 48_000, 600, 0.2), 48_000).unwrap();
        assert_eq!(p.spoken_ms, 600);
        assert_eq!(p.wav.len(), 44 + 19_200 * 2);
    }
}
