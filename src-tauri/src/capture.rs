//! The microphone. cpal's stream may not leave the thread that built it, so every recording
//! gets its own thread that owns the stream until it is told to stop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};

/// A recording stops growing here; the dictation ends it at the same limit.
pub const MAX_SECONDS: usize = 600;

pub struct Recorder {
    stop: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
    samples: Arc<Mutex<Vec<f32>>>,
    failed: Arc<AtomicBool>,
    pub rate: u32,
    pub device: String,
    /// The chosen microphone was not there; the default one is used instead.
    pub fell_back: bool,
}

pub struct Recording {
    pub samples: Vec<f32>,
    pub rate: u32,
    /// The device went away mid-recording (unplugged, driver reset).
    pub interrupted: bool,
}

/// All input devices by name, and the system default's name.
pub fn list() -> (Vec<String>, Option<String>) {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.name().ok());
    let mut names: Vec<String> = host
        .input_devices()
        .map(|devs| devs.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    names.dedup();
    (names, default)
}

impl Recorder {
    /// Opens [device] (empty = default) and starts recording. [on_level] gets 0..1 about 30
    /// times a second.
    pub fn start(device: &str, on_level: impl Fn(f32) + Send + 'static) -> Result<Recorder, String> {
        let samples = Arc::new(Mutex::new(Vec::with_capacity(48_000 * 30)));
        let failed = Arc::new(AtomicBool::new(false));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(u32, String, bool), String>>(1);

        let wanted = device.to_string();
        let buf = samples.clone();
        let fail = failed.clone();
        let thread = std::thread::Builder::new()
            .name("amber-capture".into())
            .spawn(move || {
                let stream = match open(&wanted, buf, fail, on_level) {
                    Ok((stream, rate, name, fell_back)) => {
                        let _ = ready_tx.send(Ok((rate, name, fell_back)));
                        stream
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                // Wait until told to stop (or the Recorder was dropped), then drop the stream.
                let _ = stop_rx.recv();
                let _ = stream.pause();
                drop(stream);
            })
            .map_err(|e| e.to_string())?;

        match ready_rx.recv() {
            Ok(Ok((rate, device, fell_back))) => Ok(Recorder {
                stop: stop_tx,
                thread: Some(thread),
                samples,
                failed,
                rate,
                device,
                fell_back,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("The microphone thread ended unexpectedly.".into()),
        }
    }

    pub fn finish(mut self) -> Recording {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let samples = std::mem::take(&mut *self.samples.lock().unwrap());
        Recording { samples, rate: self.rate, interrupted: self.failed.load(Ordering::Relaxed) }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn open(
    wanted: &str,
    buf: Arc<Mutex<Vec<f32>>>,
    failed: Arc<AtomicBool>,
    on_level: impl Fn(f32) + Send + 'static,
) -> Result<(cpal::Stream, u32, String, bool), String> {
    let host = cpal::default_host();
    let named = (!wanted.is_empty())
        .then(|| host.input_devices().ok()?.find(|d| d.name().map(|n| n == wanted).unwrap_or(false)))
        .flatten();
    let fell_back = !wanted.is_empty() && named.is_none();
    let device = named
        .or_else(|| host.default_input_device())
        .ok_or_else(|| "No microphone found. Plug one in or check the Windows sound settings.".to_string())?;
    let name = device.name().unwrap_or_else(|_| "Microphone".into());
    let config = device.default_input_config().map_err(|e| denied_hint(&e.to_string()))?;
    let rate = config.sample_rate().0;
    let channels = config.channels();
    let stream_config: cpal::StreamConfig = config.clone().into();

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &stream_config, channels, rate, buf, failed, on_level),
        cpal::SampleFormat::I16 => build::<i16>(&device, &stream_config, channels, rate, buf, failed, on_level),
        cpal::SampleFormat::U16 => build::<u16>(&device, &stream_config, channels, rate, buf, failed, on_level),
        cpal::SampleFormat::I32 => build::<i32>(&device, &stream_config, channels, rate, buf, failed, on_level),
        cpal::SampleFormat::U8 => build::<u8>(&device, &stream_config, channels, rate, buf, failed, on_level),
        other => return Err(format!("The microphone delivers an unsupported format ({other:?}).")),
    }
    .map_err(|e| denied_hint(&e))?;
    stream.play().map_err(|e| denied_hint(&e.to_string()))?;
    Ok((stream, rate, name, fell_back))
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: u16,
    rate: u32,
    buf: Arc<Mutex<Vec<f32>>>,
    failed: Arc<AtomicBool>,
    on_level: impl Fn(f32) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let max = rate as usize * MAX_SECONDS;
    let level_every = (rate / 30).max(1) as usize;
    let mut window: Vec<f32> = Vec::with_capacity(level_every);
    let mut mono: Vec<f32> = Vec::new();
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                mono.clear();
                let ch = channels.max(1) as usize;
                mono.extend(data.chunks_exact(ch).map(|frame| frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / ch as f32));
                {
                    let mut b = buf.lock().unwrap();
                    let room = max.saturating_sub(b.len());
                    b.extend_from_slice(&mono[..mono.len().min(room)]);
                }
                for &s in &mono {
                    window.push(s);
                    if window.len() >= level_every {
                        on_level(amber_core::audio::level(amber_core::audio::rms(&window)));
                        window.clear();
                    }
                }
            },
            move |err| {
                failed.store(true, Ordering::Relaxed);
                crate::diag::warn(format!("microphone stream error: {err}"));
            },
            None,
        )
        .map_err(|e| e.to_string())
}

/// Windows answers "access denied" when the privacy settings block desktop apps from the
/// microphone; that deserves a pointer to the right switch.
fn denied_hint(e: &str) -> String {
    let lower = e.to_lowercase();
    if lower.contains("denied") || lower.contains("0x80070005") {
        "Windows blocks the microphone. Settings → Privacy & security → Microphone → allow desktop apps.".into()
    } else {
        format!("The microphone could not be opened: {e}")
    }
}
