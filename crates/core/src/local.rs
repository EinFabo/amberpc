//! The local engine's parts: a whisper.cpp build and a model, both downloaded on demand,
//! pinned by URL and SHA-256. Nothing unverified ever lands where the app runs it from.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// whisper.cpp's prebuilt Windows binaries, release b5454 (2026-10-06).
pub const WHISPER_CPP_BUILD: &str = "b5454";

pub struct Runtime {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_mb: u32,
    /// Files the runtime cannot work without. An archive missing one is refused - whisper.cpp
    /// would otherwise start anyway and quietly fall back to the CPU.
    pub required: &'static [&'static str],
}

pub const RUNTIMES: [Runtime; 2] = [
    Runtime {
        id: "cpu",
        label: "CPU",
        note: "runs on every PC",
        url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip",
        sha256: "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
        size_mb: 8,
        required: &["whisper-server.exe", "whisper.dll", "ggml.dll", "ggml-base.dll"],
    },
    Runtime {
        id: "cuda",
        label: "NVIDIA GPU (CUDA)",
        note: "many times faster - GTX 900 or newer, driver 551 or newer",
        // The CUDA 12.4 package, not 11.8: only this one ships cuBLAS, which ggml-cuda.dll
        // needs to load at all.
        url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-win-cuda-12.4.0-x64.zip",
        sha256: "afef0b881c500958921c3f5523b50e59ee2ec9b6f5cbd25b324c51ed308a957a",
        size_mb: 653,
        required: &[
            "whisper-server.exe",
            "whisper.dll",
            "ggml.dll",
            "ggml-base.dll",
            "ggml-cuda.dll",
            "cublas64_12.dll",
            "cublasLt64_12.dll",
            "cudart64_12.dll",
        ],
    },
];

pub struct Model {
    pub id: &'static str,
    pub file: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_mb: u32,
    /// A GPU makes this one comfortable; on a CPU it is slow.
    pub wants_gpu: bool,
    /// Fine-tuned (ACFT) so the encoder may stop where the speech stops instead of always
    /// working through a 30 s window - the biggest speed-up on a CPU. A stock model loses
    /// words when its context is cut, so only these get a short one.
    pub short_context: bool,
    /// Context positions beyond the audio for a [short_context] model (see `audio_ctx_for`).
    pub context_slack: u32,
    /// Kept for existing downloads; offered only while installed.
    pub replaced: bool,
}

impl Model {
    pub fn url(&self) -> String {
        self.url.to_string()
    }
}

/// The ggerganov/whisper.cpp model repository at a fixed commit, so a file can never change
/// under its checksum.
pub const MODELS_COMMIT: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

macro_rules! ggerganov {
    ($file:literal) => {
        concat!("https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/", $file)
    };
}

/// The same models as Amber for Android 0.16, pinned by URL and SHA-256 there and here.
pub const MODELS: [Model; 8] = [
    Model {
        id: "futo-multilingual-74",
        file: "voice-input-multilingual-74.bin",
        label: "Base (FUTO)",
        note: "Whisper base, tuned for short dictations - fast on any PC, makes more mistakes",
        url: "https://keyboard.futo.org/voice-input-multilingual-74.bin",
        sha256: "e44f352c9aa2c3609dece20c733c4ad4a75c28cd9ab07d005383df55fa96efc4",
        size_mb: 78,
        wants_gpu: false,
        short_context: true,
        context_slack: 64,
        replaced: false,
    },
    Model {
        id: "futo-multilingual-244",
        file: "voice-input-multilingual-244.bin",
        label: "Small (FUTO)",
        note: "Whisper small, tuned for short dictations - good on a CPU",
        url: "https://keyboard.futo.org/voice-input-multilingual-244.bin",
        sha256: "15ef255465a6dc582ecf1ec651a4618c7ee2c18c05570bbe46493d248d465ac4",
        size_mb: 253,
        wants_gpu: false,
        short_context: true,
        context_slack: 64,
        replaced: false,
    },
    Model {
        id: "acft-large-v3-turbo-q8_0",
        file: "acft-whisper-large-v3-turbo-q8_0.bin",
        label: "Large v3 Turbo",
        note: "the large model, tuned to stop where your speech stops - most accurate, fast on a GPU",
        // MahmoudAshraf/acft-whisper-large-v3-turbo (Apache 2.0), converted by DeadBranches;
        // Amber for Android checked it tensor by tensor against its own conversion.
        url: "https://huggingface.co/DeadBranches/acft-whisper-large-v3-turbo_q8_0/resolve/609781a1dc54e208ccc91d6ab6c0c53b08ee879b/acft-whisper-large-v3-turbo-q8_0.bin",
        sha256: "6d862034d882b83f76366fefc1bd750cef2057d2c644378c46e41e90b0331a72",
        size_mb: 834,
        wants_gpu: true,
        short_context: true,
        // As its model card recommends; on Android 4.53 % WER with 128, 4.64 % with 64.
        context_slack: 128,
        replaced: false,
    },
    Model {
        id: "base-q5_1",
        file: "ggml-base-q5_1.bin",
        label: "Base",
        note: "replaced by Base (FUTO)",
        url: ggerganov!("ggml-base-q5_1.bin"),
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        size_mb: 57,
        wants_gpu: false,
        short_context: false,
        context_slack: 0,
        replaced: true,
    },
    Model {
        id: "small-q5_1",
        file: "ggml-small-q5_1.bin",
        label: "Small",
        note: "replaced by Small (FUTO)",
        url: ggerganov!("ggml-small-q5_1.bin"),
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        size_mb: 182,
        wants_gpu: false,
        short_context: false,
        context_slack: 0,
        replaced: true,
    },
    Model {
        id: "large-v3-turbo-q5_0",
        file: "ggml-large-v3-turbo-q5_0.bin",
        label: "Large v3 Turbo (old)",
        note: "always works through 30 s - the tuned Large v3 Turbo is as accurate and faster",
        url: ggerganov!("ggml-large-v3-turbo-q5_0.bin"),
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        size_mb: 548,
        wants_gpu: true,
        short_context: false,
        context_slack: 0,
        replaced: true,
    },
    Model {
        id: "large-v3-turbo-q8_0",
        file: "ggml-large-v3-turbo-q8_0.bin",
        label: "Large v3 Turbo q8 (old)",
        note: "replaced by the tuned Large v3 Turbo",
        url: ggerganov!("ggml-large-v3-turbo-q8_0.bin"),
        sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
        size_mb: 834,
        wants_gpu: true,
        short_context: false,
        context_slack: 0,
        replaced: true,
    },
    Model {
        id: "large-v3-q5_0",
        file: "ggml-large-v3-q5_0.bin",
        label: "Large v3 (old)",
        note: "slowest; the tuned Large v3 Turbo is about as accurate",
        url: ggerganov!("ggml-large-v3-q5_0.bin"),
        sha256: "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        size_mb: 1_031,
        wants_gpu: true,
        short_context: false,
        context_slack: 0,
        replaced: true,
    },
];

/// Whisper's encoder sees 30 s at most, as 1500 positions: one per 320 samples at 16 kHz.
const FULL_AUDIO_CTX: u32 = 1500;
const SAMPLES_PER_CTX: usize = 320;
/// Below this, a shortened context starts to hurt more than it saves.
const MIN_AUDIO_CTX: u32 = 256;

/// How much of the encoder context a short-context model needs for [samples] at 16 kHz:
/// the audio plus [slack], at least 256 - or 0 (the full window) from 30 s on, where
/// whisper.cpp slides over the audio window by window anyway. Port of Android's
/// `WhisperInput.audioCtxFor`.
pub fn audio_ctx_for(samples: usize, slack: u32) -> u32 {
    let needed = samples.div_ceil(SAMPLES_PER_CTX) as u32 + slack;
    if needed >= FULL_AUDIO_CTX {
        0
    } else {
        needed.max(MIN_AUDIO_CTX)
    }
}

/// One download of a package.
pub struct Archive {
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_mb: u32,
}

/// llama.cpp's prebuilt Windows binaries for the smart cleanup, release b11981 (2026-10-29).
pub const LLAMA_CPP_BUILD: &str = "b11981";
pub const LLM_SERVER_EXE: &str = "llama-server.exe";

pub struct LlmRuntime {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub archives: &'static [Archive],
    pub required: &'static [&'static str],
}

impl LlmRuntime {
    pub fn size_mb(&self) -> u32 {
        self.archives.iter().map(|a| a.size_mb).sum()
    }
    /// What `.installed` holds: every archive's checksum, so a changed package reinstalls.
    pub fn marker(&self) -> String {
        self.archives.iter().map(|a| a.sha256).collect::<Vec<_>>().join(",")
    }
}

pub const LLM_RUNTIMES: [LlmRuntime; 2] = [
    LlmRuntime {
        id: "cpu",
        label: "CPU",
        note: "slow - a few seconds per sentence on a fast CPU",
        archives: &[Archive {
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b11981/llama-b11981-bin-win-cpu-x64.zip",
            sha256: "626dd4136947fb9cdf755523fefcad3d414858a4beca06ce424fcd3ee0a948c9",
            size_mb: 23,
        }],
        required: &["llama-server.exe", "llama-server-impl.dll", "llama.dll", "ggml.dll", "ggml-base.dll"],
    },
    LlmRuntime {
        id: "cuda",
        label: "NVIDIA GPU (CUDA)",
        note: "well under a second per sentence",
        // The CUDA build and, separately, the CUDA runtime with cuBLAS it needs to load.
        archives: &[
            Archive {
                url: "https://github.com/ggml-org/llama.cpp/releases/download/b11981/llama-b11981-bin-win-cuda-12.4-x64.zip",
                sha256: "dcb0f6be103aec6c55a365be71a678c0cf620f9c85bf9a16de78090edd44ddbb",
                size_mb: 267,
            },
            Archive {
                url: "https://github.com/ggml-org/llama.cpp/releases/download/b11981/cudart-llama-bin-win-cuda-12.4-x64.zip",
                sha256: "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6",
                size_mb: 374,
            },
        ],
        required: &[
            "llama-server.exe",
            "llama-server-impl.dll",
            "llama.dll",
            "ggml.dll",
            "ggml-base.dll",
            "ggml-cuda.dll",
            "cublas64_12.dll",
            "cublasLt64_12.dll",
            "cudart64_12.dll",
        ],
    },
];

pub struct LlmModel {
    pub id: &'static str,
    pub file: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_mb: u32,
}

/// Apache 2.0, no thinking mode (no time spent before the answer), good at German.
pub const LLM_MODELS: [LlmModel; 1] = [LlmModel {
    id: "qwen3-4b-instruct-2507-q4km",
    file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
    label: "Qwen3 4B Instruct",
    note: "good German, about 3 GB of graphics memory",
    url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/a06e946bb6b655725eafa393f4a9745d460374c9/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
    sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
    size_mb: 2_382,
}];

pub fn llm_runtime(id: &str) -> Option<&'static LlmRuntime> {
    LLM_RUNTIMES.iter().find(|r| r.id == id)
}

pub fn llm_model(id: &str) -> Option<&'static LlmModel> {
    LLM_MODELS.iter().find(|m| m.id == id)
}

pub fn runtime(id: &str) -> Option<&'static Runtime> {
    RUNTIMES.iter().find(|r| r.id == id)
}

pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

/// What to suggest: the GPU build with the tuned large model where there is an NVIDIA card;
/// else the tuned small model on a CPU with room (8+ threads, 8+ GB), the tuned base model on
/// anything weaker (an old 4-core office PC with 4 GB). Only ever a suggestion.
pub fn recommend(has_nvidia: bool, threads: u32, ram_mb: u64) -> (&'static str, &'static str) {
    if has_nvidia {
        ("cuda", "acft-large-v3-turbo-q8_0")
    } else if threads >= 8 && ram_mb >= 7_500 {
        ("cpu", "futo-multilingual-244")
    } else {
        ("cpu", "futo-multilingual-74")
    }
}

/// Where downloads live: `<root>/runtime/<id>/` and `<root>/models/<file>`.
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
}

/// The one program the app starts from a runtime.
pub const SERVER_EXE: &str = "whisper-server.exe";

impl Layout {
    pub fn runtime_dir(&self, rt: &Runtime) -> PathBuf {
        self.root.join("runtime").join(rt.id)
    }
    pub fn server_exe(&self, rt: &Runtime) -> PathBuf {
        self.runtime_dir(rt).join(SERVER_EXE)
    }
    pub fn model_path(&self, m: &Model) -> PathBuf {
        self.root.join("models").join(m.file)
    }
    /// A runtime counts as installed once its marker is there - written last, after every
    /// file was extracted.
    pub fn runtime_installed(&self, rt: &Runtime) -> bool {
        let dir = self.runtime_dir(rt);
        fs::read_to_string(dir.join(".installed")).map(|v| v.trim() == rt.sha256).unwrap_or(false)
            && rt.required.iter().all(|f| dir.join(f).is_file())
    }
    pub fn model_installed(&self, m: &Model) -> bool {
        fs::metadata(self.model_path(m)).map(|md| md.len() > 1_000_000).unwrap_or(false)
    }
    pub fn llm_runtime_dir(&self, rt: &LlmRuntime) -> PathBuf {
        self.root.join("llm").join("runtime").join(rt.id)
    }
    pub fn llm_server_exe(&self, rt: &LlmRuntime) -> PathBuf {
        self.llm_runtime_dir(rt).join(LLM_SERVER_EXE)
    }
    pub fn llm_model_path(&self, m: &LlmModel) -> PathBuf {
        self.root.join("llm").join("models").join(m.file)
    }
    pub fn llm_runtime_installed(&self, rt: &LlmRuntime) -> bool {
        let dir = self.llm_runtime_dir(rt);
        fs::read_to_string(dir.join(".installed")).map(|v| v.trim() == rt.marker()).unwrap_or(false)
            && rt.required.iter().all(|f| dir.join(f).is_file())
    }
    pub fn llm_model_installed(&self, m: &LlmModel) -> bool {
        fs::metadata(self.llm_model_path(m)).map(|md| md.len() > 1_000_000).unwrap_or(false)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download cancelled")]
    Cancelled,
    #[error("download failed: {0}")]
    Network(String),
    #[error("the download is damaged (checksum mismatch)")]
    Checksum,
    #[error("could not write the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("the package is unusable: {0}")]
    Package(String),
}

/// Streams [url] into [dest], hashing on the way. Only a file with the right SHA-256 is moved
/// into place; anything else is deleted. [progress] gets (bytes so far, total if known).
pub fn download_verified(
    url: &str,
    sha256: &str,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), DownloadError> {
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let part = dest.with_extension("part");
    let result = (|| {
        // In the blocking client the timeout applies to every single read: a connection that
        // stalls ends the download (and lets a cancel through) instead of hanging forever.
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(45))
            .user_agent(concat!("AmberPC/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| DownloadError::Network(e.to_string()))?;
        let mut resp = client.get(url).send().map_err(|e| DownloadError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(DownloadError::Network(format!("HTTP {}", resp.status().as_u16())));
        }
        let total = resp.content_length();
        let mut file = File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        let mut last_report = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
            let n = resp.read(&mut buf).map_err(|e| DownloadError::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            done += n as u64;
            if done - last_report >= 1 << 20 {
                last_report = done;
                progress(done, total);
            }
        }
        file.sync_all()?;
        drop(file);
        progress(done, total);
        if !hex(&hasher.finalize()).eq_ignore_ascii_case(sha256) {
            return Err(DownloadError::Checksum);
        }
        fs::rename(&part, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

/// Unpacks what the app needs from a whisper.cpp release zip - the server and every DLL,
/// none of the test programs - flat into [dest].
pub fn install_runtime(zip_path: &Path, dest: &Path, rt: &Runtime) -> Result<(), DownloadError> {
    install_archives(&[zip_path], dest, SERVER_EXE, rt.required, rt.sha256)
}

/// The same for llama.cpp, whose CUDA build comes in two archives.
pub fn install_llm_runtime(zips: &[&Path], dest: &Path, rt: &LlmRuntime) -> Result<(), DownloadError> {
    install_archives(zips, dest, LLM_SERVER_EXE, rt.required, &rt.marker())
}

/// Extracts [server_exe] and every DLL from [zips] flat into a sibling folder of [dest],
/// checks that every [required] file is there, writes [marker] last and only then swaps the
/// folder in - a failed unpack never leaves half a runtime behind.
fn install_archives(zips: &[&Path], dest: &Path, server_exe: &str, required: &[&str], marker: &str) -> Result<(), DownloadError> {
    let staging = dest.with_extension("new");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let result = (|| {
        let mut found: Vec<String> = Vec::new();
        for zip_path in zips {
            let mut archive = zip::ZipArchive::new(File::open(zip_path)?).map_err(|e| DownloadError::Package(e.to_string()))?;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).map_err(|e| DownloadError::Package(e.to_string()))?;
                // enclosed_name refuses "..", absolute paths and the like.
                let Some(path) = entry.enclosed_name() else { continue };
                let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
                if !entry.is_file() || !wanted(&name, server_exe) {
                    continue;
                }
                found.push(name.to_ascii_lowercase());
                let mut out = File::create(staging.join(&name))?;
                std::io::copy(&mut entry, &mut out)?;
            }
        }
        let missing: Vec<&str> = required.iter().copied().filter(|f| !found.contains(&f.to_ascii_lowercase())).collect();
        if !missing.is_empty() {
            return Err(DownloadError::Package(format!("missing {}", missing.join(", "))));
        }
        fs::write(staging.join(".installed"), marker)?;
        let _ = fs::remove_dir_all(dest);
        fs::rename(&staging, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// What whisper-server's start-up output says about the GPU: `Some(true)` once it uses a
/// CUDA backend or put the weights on CUDA0, `Some(false)` when it found no GPU or could not
/// start the backend, `None` when it has not said yet. Merely listing a device proves nothing.
pub fn gpu_in_use<'a>(output: impl IntoIterator<Item = &'a str>) -> Option<bool> {
    for line in output {
        let l = line.to_ascii_lowercase();
        if l.contains("no gpu found") || l.contains("failed to initialize") {
            return Some(false);
        }
        if l.contains("using cuda") || l.contains("cuda0 total size") {
            return Some(true);
        }
    }
    None
}

fn wanted(name: &str, server_exe: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == server_exe || lower.ends_with(".dll")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a file on disk, for checks after the fact.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut f, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testserver::serve;

    #[test]
    fn catalog_is_consistent() {
        for r in &RUNTIMES {
            assert_eq!(r.sha256.len(), 64);
            assert!(r.url.starts_with("https://github.com/ggml-org/whisper.cpp/releases/download/"));
            assert!(r.url.contains(WHISPER_CPP_BUILD));
        }
        for m in &MODELS {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.url.starts_with("https://") && m.url.ends_with(m.file), "{}", m.id);
            assert_eq!(model(m.id).map(|x| x.file), Some(m.file));
            assert_eq!(m.short_context, m.context_slack > 0, "{}", m.id);
            assert!(!(m.replaced && m.short_context), "the tuned models are the current ones");
            if m.url.contains("huggingface.co") {
                assert!(m.url.contains("/resolve/") && !m.url.contains("/resolve/main/"), "pinned: {}", m.id);
            }
        }
        for (gpu, threads, ram) in [(true, 20, 32_000), (false, 20, 32_000), (false, 8, 4_000), (false, 4, 16_000)] {
            let (rt, m) = recommend(gpu, threads, ram);
            let m = model(m).unwrap();
            assert!(runtime(rt).is_some() && m.short_context && !m.replaced);
            assert_eq!(m.wants_gpu, gpu);
        }
        assert_eq!(recommend(false, 20, 32_000).1, "futo-multilingual-244");
        // Fabi's OptiPlex 790: 4 cores / 8 threads, 4 GB.
        assert_eq!(recommend(false, 8, 3_977).1, "futo-multilingual-74");
    }

    #[test]
    fn audio_context_follows_the_speech() {
        assert_eq!(audio_ctx_for(19_200, 64), 256, "1.2 s padded: the floor");
        assert_eq!(audio_ctx_for(80_000, 64), 314, "5 s");
        assert_eq!(audio_ctx_for(128_000, 128), 528, "8 s with the large model's slack");
        assert_eq!(audio_ctx_for(80_001, 64), 315, "a started position counts");
        assert_eq!(audio_ctx_for(16_000 * 29, 128), 0, "near 30 s: the full window");
        assert_eq!(audio_ctx_for(16_000 * 60, 64), 0, "longer: whisper.cpp slides anyway");
    }

    #[test]
    fn llm_catalog_is_consistent() {
        for r in &LLM_RUNTIMES {
            assert!(!r.archives.is_empty());
            for a in r.archives {
                assert_eq!(a.sha256.len(), 64);
                assert!(a.url.starts_with("https://github.com/ggml-org/llama.cpp/releases/download/b11981/"));
            }
            assert!(r.required.contains(&LLM_SERVER_EXE));
            assert_eq!(llm_runtime(r.id).map(|x| x.id), Some(r.id));
        }
        let cuda = llm_runtime("cuda").unwrap();
        assert!(cuda.required.contains(&"cublas64_12.dll"));
        assert_eq!(cuda.size_mb(), 641);
        for m in &LLM_MODELS {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.url.ends_with(m.file) && m.url.contains("/resolve/a06e946b"));
        }
    }

    #[test]
    fn llm_runtime_from_two_archives() {
        let dir = tempfile::tempdir().unwrap();
        let cuda = llm_runtime("cuda").unwrap();
        let main_zip = dir.path().join("llama.zip");
        let cudart_zip = dir.path().join("cudart.zip");
        let main: Vec<(&str, &[u8])> =
            cuda.required.iter().filter(|f| !f.starts_with("cu")).map(|f| (*f, b"x" as &[u8])).chain([("llama-cli.exe", b"no" as &[u8])]).collect();
        make_zip(&main_zip, &main);
        let layout = Layout { root: dir.path().join("data") };
        let dest = layout.llm_runtime_dir(cuda);
        // The main archive alone lacks cuBLAS.
        assert!(install_llm_runtime(&[main_zip.as_path()], &dest, cuda).unwrap_err().to_string().contains("cublas"));
        make_zip(&cudart_zip, &[("cublas64_12.dll", b"x"), ("cublasLt64_12.dll", b"x"), ("cudart64_12.dll", b"x")]);
        install_llm_runtime(&[main_zip.as_path(), cudart_zip.as_path()], &dest, cuda).unwrap();
        assert!(layout.llm_runtime_installed(cuda));
        assert!(!dest.join("llama-cli.exe").exists());
    }

    #[test]
    fn reads_the_backend_from_the_server_output() {
        let gpu = ["whisper_init_with_params_no_state: use gpu    = 1", "whisper_backend_init_gpu: using CUDA0 backend"];
        assert_eq!(gpu_in_use(gpu), Some(true));
        let fallback = ["whisper_init_with_params_no_state: use gpu    = 1", "whisper_backend_init_gpu: no GPU found"];
        assert_eq!(gpu_in_use(fallback), Some(false));
        assert_eq!(gpu_in_use(["whisper_model_load: loading model"]), None);
        // What b5454 printed on Fabi's RTX 3070 when the "using" line had not come yet: the
        // weights on CUDA0 are proof enough.
        let loaded = [
            "whisper_backend_init_gpu: device 0: CUDA0 (type: 1)",
            "whisper_model_load: n_langs       = 100",
            "whisper_model_load:        CUDA0 total size =   573.45 MB",
        ];
        assert_eq!(gpu_in_use(loaded), Some(true));
        assert_eq!(gpu_in_use(["whisper_model_load:        CUDA0 total size =   573.45 MB"]), Some(true));
        // Listed, then failed to start: the CPU does the work.
        let failed = ["whisper_backend_init_gpu: device 0: CUDA0 (type: 1)", "whisper_backend_init_gpu: failed to initialize CUDA0 backend"];
        assert_eq!(gpu_in_use(failed), Some(false));
        assert_eq!(gpu_in_use(["whisper_backend_init_gpu: device 0: CUDA0 (type: 1)"]), None);
    }

    #[test]
    fn download_checks_the_hash() {
        let body = b"model bytes".to_vec();
        let good = hex(&Sha256::digest(&body));
        let (url, _rx) = serve(vec![(200, "application/octet-stream", body.clone()), (200, "application/octet-stream", body.clone())]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("models").join("m.bin");
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();

        let err = download_verified(&url, &"0".repeat(64), &dest, &cancel, &mut |d, t| seen.push((d, t))).unwrap_err();
        assert!(matches!(err, DownloadError::Checksum));
        assert!(!dest.exists() && !dest.with_extension("part").exists());

        download_verified(&url, &good, &dest, &cancel, &mut |d, t| seen.push((d, t))).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert_eq!(seen.last(), Some(&(11, Some(11))));
        assert_eq!(sha256_file(&dest).unwrap(), good);
    }

    #[test]
    fn download_reports_http_errors_and_cancel() {
        let (url, _rx) = serve(vec![(404, "text/plain", b"no".to_vec()), (200, "application/octet-stream", vec![1; 10])]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("x.bin");
        let cancel = AtomicBool::new(false);
        assert!(matches!(download_verified(&url, "", &dest, &cancel, &mut |_, _| {}), Err(DownloadError::Network(_))));
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(download_verified(&url, "", &dest, &cancel, &mut |_, _| {}), Err(DownloadError::Cancelled)));
        assert!(!dest.exists());
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn runtime_install_takes_server_and_dlls_only() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("rt.zip");
        make_zip(&zip_path, &[
            ("Release/whisper-server.exe", b"exe"),
            ("Release/whisper.dll", b"dll"),
            ("Release/ggml.dll", b"dll"),
            ("Release/ggml-base.dll", b"dll"),
            ("Release/ggml-cpu-alderlake.dll", b"dll"),
            ("Release/test-vad.exe", b"no"),
            ("../evil.dll", b"no"),
        ]);
        let layout = Layout { root: dir.path().join("data") };
        let rt = &RUNTIMES[0];
        assert!(!layout.runtime_installed(rt));
        install_runtime(&zip_path, &layout.runtime_dir(rt), rt).unwrap();
        assert!(layout.runtime_installed(rt));
        let mut names: Vec<_> = fs::read_dir(layout.runtime_dir(rt)).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names, [".installed", "ggml-base.dll", "ggml-cpu-alderlake.dll", "ggml.dll", "whisper-server.exe", "whisper.dll"]);
        // A required file gone afterwards (antivirus, manual cleanup) means not installed.
        fs::remove_file(layout.runtime_dir(rt).join("ggml.dll")).unwrap();
        assert!(!layout.runtime_installed(rt));
        assert!(!dir.path().join("evil.dll").exists() && !dir.path().join("data/runtime/evil.dll").exists());
    }

    #[test]
    fn cuda_runtime_without_cublas_is_refused() {
        // whisper.cpp's CUDA 11.8 package shipped ggml-cuda.dll without the cuBLAS it links to:
        // Windows then fails to load the GPU backend and whisper.cpp silently runs on the CPU.
        let cuda = runtime("cuda").unwrap();
        assert!(cuda.required.iter().any(|f| f.starts_with("cublas64_")));
        assert!(cuda.required.iter().any(|f| f.starts_with("cublasLt64_")));
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("rt.zip");
        let present: Vec<(String, &[u8])> = cuda
            .required
            .iter()
            .filter(|f| !f.starts_with("cublas"))
            .map(|f| (format!("Release/{f}"), b"x" as &[u8]))
            .collect();
        let entries: Vec<(&str, &[u8])> = present.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        make_zip(&zip_path, &entries);
        let dest = dir.path().join("runtime/cuda");
        let err = install_runtime(&zip_path, &dest, cuda).unwrap_err();
        assert!(err.to_string().contains("cublas"), "{err}");
        assert!(!dest.exists());
    }

    #[test]
    fn runtime_without_server_is_refused_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("rt.zip");
        make_zip(&zip_path, &[("Release/whisper.dll", b"dll")]);
        let dest = dir.path().join("runtime/cpu");
        assert!(matches!(install_runtime(&zip_path, &dest, &RUNTIMES[0]), Err(DownloadError::Package(_))));
        assert!(!dest.exists() && !dest.with_extension("new").exists());
    }
}
