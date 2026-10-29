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
}

pub const RUNTIMES: [Runtime; 2] = [
    Runtime {
        id: "cpu",
        label: "CPU",
        note: "runs on every PC",
        url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip",
        sha256: "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
        size_mb: 8,
    },
    Runtime {
        id: "cuda",
        label: "NVIDIA GPU (CUDA)",
        note: "many times faster - GTX 900 or newer with a current driver",
        url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-win-cuda-11.8.0-x64.zip",
        sha256: "e81c86c542be6e3d7b7245392d4e1c52ab6e5c61dbe38721754bc0ab59d4d027",
        size_mb: 272,
    },
];

pub struct Model {
    pub id: &'static str,
    pub file: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub sha256: &'static str,
    pub size_mb: u32,
    /// A GPU makes this one comfortable; on a CPU it is slow.
    pub wants_gpu: bool,
}

impl Model {
    pub fn url(&self) -> String {
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}", self.file)
    }
}

pub const MODELS: [Model; 5] = [
    Model {
        id: "base-q5_1",
        file: "ggml-base-q5_1.bin",
        label: "Base",
        note: "tiny and quick, makes more mistakes",
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        size_mb: 57,
        wants_gpu: false,
    },
    Model {
        id: "small-q5_1",
        file: "ggml-small-q5_1.bin",
        label: "Small",
        note: "good on a CPU",
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        size_mb: 182,
        wants_gpu: false,
    },
    Model {
        id: "large-v3-turbo-q5_0",
        file: "ggml-large-v3-turbo-q5_0.bin",
        label: "Large v3 Turbo",
        note: "near the best, fast on a GPU",
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        size_mb: 548,
        wants_gpu: true,
    },
    Model {
        id: "large-v3-turbo-q8_0",
        file: "ggml-large-v3-turbo-q8_0.bin",
        label: "Large v3 Turbo (q8)",
        note: "a touch more accurate, more memory",
        sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
        size_mb: 834,
        wants_gpu: true,
    },
    Model {
        id: "large-v3-q5_0",
        file: "ggml-large-v3-q5_0.bin",
        label: "Large v3",
        note: "most accurate, slowest",
        sha256: "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        size_mb: 1_031,
        wants_gpu: true,
    },
];

pub fn runtime(id: &str) -> Option<&'static Runtime> {
    RUNTIMES.iter().find(|r| r.id == id)
}

pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

/// What to suggest on first start: the GPU build with Turbo where there is an NVIDIA card,
/// otherwise the CPU with Small.
pub fn recommend(has_nvidia: bool) -> (&'static str, &'static str) {
    if has_nvidia {
        ("cuda", "large-v3-turbo-q5_0")
    } else {
        ("cpu", "small-q5_1")
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
        fs::read_to_string(self.runtime_dir(rt).join(".installed")).map(|v| v.trim() == rt.sha256).unwrap_or(false)
            && self.server_exe(rt).is_file()
    }
    pub fn model_installed(&self, m: &Model) -> bool {
        fs::metadata(self.model_path(m)).map(|md| md.len() > 1_000_000).unwrap_or(false)
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
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(None)
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
/// none of the test programs - flat into [dest]. Extracts into a sibling folder first and
/// swaps it in, so a failed unpack never leaves half a runtime behind.
pub fn install_runtime(zip_path: &Path, dest: &Path, sha256: &str) -> Result<(), DownloadError> {
    let staging = dest.with_extension("new");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let result = (|| {
        let mut archive = zip::ZipArchive::new(File::open(zip_path)?).map_err(|e| DownloadError::Package(e.to_string()))?;
        let mut found_server = false;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| DownloadError::Package(e.to_string()))?;
            // enclosed_name refuses "..", absolute paths and the like.
            let Some(path) = entry.enclosed_name() else { continue };
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            if !entry.is_file() || !wanted(&name) {
                continue;
            }
            found_server |= name.eq_ignore_ascii_case(SERVER_EXE);
            let mut out = File::create(staging.join(&name))?;
            std::io::copy(&mut entry, &mut out)?;
        }
        if !found_server {
            return Err(DownloadError::Package(format!("{SERVER_EXE} missing")));
        }
        fs::write(staging.join(".installed"), sha256)?;
        let _ = fs::remove_dir_all(dest);
        fs::rename(&staging, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn wanted(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == SERVER_EXE || lower.ends_with(".dll")
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
            assert!(m.url().ends_with(m.file));
            assert_eq!(model(m.id).map(|x| x.file), Some(m.file));
        }
        let (rt, m) = recommend(true);
        assert!(runtime(rt).is_some() && model(m).is_some());
        let (rt, m) = recommend(false);
        assert!(runtime(rt).is_some() && !model(m).unwrap().wants_gpu);
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
            ("Release/ggml-cpu-alderlake.dll", b"dll"),
            ("Release/test-vad.exe", b"no"),
            ("../evil.dll", b"no"),
        ]);
        let layout = Layout { root: dir.path().join("data") };
        let rt = &RUNTIMES[0];
        assert!(!layout.runtime_installed(rt));
        install_runtime(&zip_path, &layout.runtime_dir(rt), rt.sha256).unwrap();
        assert!(layout.runtime_installed(rt));
        let mut names: Vec<_> = fs::read_dir(layout.runtime_dir(rt)).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names, [".installed", "ggml-cpu-alderlake.dll", "whisper-server.exe", "whisper.dll"]);
        assert!(!dir.path().join("evil.dll").exists() && !dir.path().join("data/runtime/evil.dll").exists());
    }

    #[test]
    fn runtime_without_server_is_refused_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("rt.zip");
        make_zip(&zip_path, &[("Release/whisper.dll", b"dll")]);
        let dest = dir.path().join("runtime/cpu");
        assert!(matches!(install_runtime(&zip_path, &dest, "x"), Err(DownloadError::Package(_))));
        assert!(!dest.exists() && !dest.with_extension("new").exists());
    }
}
