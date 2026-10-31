//! What can be downloaded, what is, and how well it suits this machine - for `models`,
//! `cleanup` and `setup`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use amber_core::local::{self, LlmModel, Model};

use crate::config::{Config, Gpu};
use crate::paths::{cleanup_dir, data_dir, llm_ready, model_ready, speech_dir};

/// What this machine brings for the models.
#[derive(Clone, Copy, Debug)]
pub struct Machine {
    /// A Vulkan build with the GPU not switched off.
    pub gpu: bool,
    pub threads: u32,
    pub ram_mb: u64,
}

pub fn machine(config: &Config) -> Machine {
    Machine {
        gpu: cfg!(feature = "vulkan") && config.gpu == Gpu::Auto,
        threads: std::thread::available_parallelism().map_or(4, |n| n.get() as u32),
        ram_mb: total_ram_mb(),
    }
}

/// The machine's memory in MB; unknown counts as 4 GB - rather a smaller model than one that
/// does not fit.
pub fn total_ram_mb() -> u64 {
    #[cfg(target_os = "linux")]
    let mb = std::fs::read_to_string("/proc/meminfo").ok().and_then(|t| meminfo_mb(&t));
    #[cfg(windows)]
    let mb = {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        unsafe { GlobalMemoryStatusEx(&mut status) }.ok().map(|_| status.ullTotalPhys / (1024 * 1024))
    };
    #[cfg(not(any(target_os = "linux", windows)))]
    let mb: Option<u64> = None;
    mb.unwrap_or(4_096)
}

/// `MemTotal` of /proc/meminfo, in MB.
fn meminfo_mb(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// The desktop's recommendation - but never a model too big for the memory (with a GPU the
/// recommendation does not look at it): then the largest small one that fits.
pub fn recommended_speech(gpu: bool, threads: u32, ram_mb: u64) -> &'static str {
    let pick = local::recommend(gpu, threads, ram_mb).1;
    std::iter::once(pick)
        .chain(["futo-multilingual-244", "futo-multilingual-74"])
        .find(|id| local::model(id).is_some_and(|m| !too_big(m, ram_mb)))
        .unwrap_or("futo-multilingual-74")
}

/// In memory a model takes about twice its file; more than half the RAM leaves too little.
fn too_big(m: &Model, ram_mb: u64) -> bool {
    u64::from(m.size_mb) * 2 > ram_mb / 2
}

/// How a speech model suits a machine: "recommended", "fits", "slow here" or "too big".
pub fn fit(m: &Model, gpu: bool, threads: u32, ram_mb: u64) -> &'static str {
    if too_big(m, ram_mb) {
        "too big"
    } else if m.id == recommended_speech(gpu, threads, ram_mb) {
        "recommended"
    } else if m.wants_gpu && !gpu {
        "slow here"
    } else {
        "fits"
    }
}

/// The default model is not deleted from under the server.
pub fn removable(default: Option<&str>, id: &str, command: &str) -> Result<(), String> {
    if default == Some(id) {
        let off = if command == "cleanup" { " (or: amber-server cleanup off)" } else { "" };
        return Err(format!("{id} is the default - choose another first: amber-server {command} default <id>{off}"));
    }
    Ok(())
}

/// A pinned download with a progress line on stderr.
fn download(url: &str, sha256: &str, dest: &Path, label: &str) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    let mut last = u64::MAX;
    let result = local::download_verified(url, sha256, dest, &cancel, &mut |done, total| {
        // Percent of a known size, else megabytes so far.
        let step = total.map_or(done >> 20, |t| done * 100 / t.max(1));
        if step != last {
            last = step;
            match total {
                Some(t) => eprint!("\r  {label}: {step}% of {} MB   ", t >> 20),
                None => eprint!("\r  {label}: {step} MB   "),
            }
            let _ = std::io::stderr().flush();
        }
    });
    eprintln!();
    result.map_err(|e| format!("{label}: {e}"))
}

pub fn speech_model(id: &str) -> Result<&'static Model, String> {
    local::model(id).ok_or_else(|| format!("no speech model \"{id}\" - see: amber-server models"))
}

pub fn cleanup_model(id: &str) -> Result<&'static LlmModel, String> {
    local::llm_model(id).ok_or_else(|| format!("no cleanup model \"{id}\" - see: amber-server cleanup"))
}

pub fn download_speech(id: &str) -> Result<(), String> {
    let m = speech_model(id)?;
    if model_ready(&speech_dir(), m) {
        return Ok(());
    }
    download(&m.url(), m.sha256, &speech_dir().join(m.file), m.label)
}

pub fn download_cleanup(id: &str, gpu: bool) -> Result<(), String> {
    let m = cleanup_model(id)?;
    ensure_llama(&data_dir(), gpu)?;
    if llm_ready(&cleanup_dir(), m) {
        return Ok(());
    }
    download(m.url, m.sha256, &cleanup_dir().join(m.file), m.label)
}

/// llama-server for this system and backend, downloaded and unpacked the first time.
pub fn ensure_llama(data: &Path, gpu: bool) -> Result<PathBuf, String> {
    let os = if cfg!(windows) { "windows" } else { "linux" };
    let backend = if gpu { "vulkan" } else { "cpu" };
    let build = local::llama_build(os, backend).ok_or_else(|| format!("no cleanup engine for {os}/{backend}"))?;
    let dir = data.join("llama").join(format!("{}-{backend}", local::LLAMA_CPP_BUILD));
    let exe = dir.join(build.exe);
    if exe.is_file() {
        return Ok(exe);
    }
    let file = build.url.rsplit('/').next().unwrap_or("llama.zip");
    let archive = data.join("downloads").join(file);
    download(build.url, build.sha256, &archive, "cleanup engine (llama.cpp)")?;
    let unpacked = local::unpack(&archive, &dir).map_err(|e| format!("cleanup engine: {e}"));
    let _ = std::fs::remove_file(&archive);
    unpacked?;
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(format!("cleanup engine: {} missing in the archive", build.exe))
    }
}

/// The speech models: downloaded ones (the default marked), then what else there is.
pub fn list_speech(config: &Config, m: Machine) -> String {
    let dir = speech_dir();
    let mut out = String::from("Speech models (* default)\n");
    for model in local::MODELS.iter().filter(|x| model_ready(&dir, x) || !x.replaced) {
        let mark = if config.speech == model.id { "*" } else { " " };
        let state = if model_ready(&dir, model) { "downloaded".to_string() } else { fit(model, m.gpu, m.threads, m.ram_mb).to_string() };
        out += &format!("{mark} {:<28} {:<24} {:>6} MB  {state}\n", model.id, model.label, model.size_mb);
    }
    out + "\nadd: amber-server models add <id>   default: amber-server models default <id>"
}

/// The cleanup models likewise.
pub fn list_cleanup(config: &Config) -> String {
    let dir = cleanup_dir();
    let mut out = String::from("Cleanup models (* default)\n");
    for model in local::LLM_MODELS.iter() {
        let mark = if config.cleanup.as_deref() == Some(model.id) { "*" } else { " " };
        let state = if llm_ready(&dir, model) { "downloaded" } else { "" };
        out += &format!("{mark} {:<32} {:<24} {:>6} MB  {state}\n", model.id, model.label, model.size_mb);
    }
    if config.cleanup.is_none() {
        out += "\nThe cleanup is off.";
    }
    out + "\nadd: amber-server cleanup add <id>   default: amber-server cleanup default <id>   off: amber-server cleanup off"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_tells_the_machine_apart() {
        let big = amber_core::local::model("large-v3-q5_0").unwrap();
        let small = amber_core::local::model("futo-multilingual-74").unwrap();
        assert_eq!(fit(big, false, 4, 4_000), "too big");
        assert_eq!(fit(small, false, 4, 4_000), "recommended");
        let turbo = amber_core::local::model("acft-large-v3-turbo-q8_0").unwrap();
        assert_eq!(fit(turbo, true, 16, 32_000), "recommended");
        assert_eq!(fit(turbo, false, 16, 32_000), "slow here", "a GPU model on a CPU");
        assert_eq!(fit(amber_core::local::model("futo-multilingual-244").unwrap(), true, 16, 32_000), "fits");
        // A GPU in a small box: the big model is too big, and not what setup takes.
        assert_eq!(fit(turbo, true, 16, 3_000), "too big");
        assert_eq!(recommended_speech(true, 16, 3_000), "futo-multilingual-244");
    }

    #[test]
    fn memory_is_read_from_proc_meminfo() {
        assert_eq!(meminfo_mb("MemTotal:       16314368 kB\nMemFree:  1 kB\n"), Some(15_932));
        assert_eq!(meminfo_mb("garbage"), None);
        assert!(total_ram_mb() > 0);
    }

    #[test]
    fn the_default_cannot_be_removed() {
        let err = removable(Some("futo-multilingual-74"), "futo-multilingual-74", "models").unwrap_err();
        assert!(err.contains("amber-server models default"), "{err}");
        assert!(removable(Some("futo-multilingual-74"), "futo-multilingual-244", "models").is_ok());
        assert!(removable(None, "qwen3-4b-instruct-2507-q4km", "cleanup").is_ok());
    }
}
