//! Helper processes (whisper-server, llama-server): started without a console window, their
//! output kept for diagnostics, and tied to Amber by one Windows job object, so none of them
//! outlives the app - not even after a crash.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// The last lines a helper printed - the reason when it dies.
pub type Output = Arc<Mutex<VecDeque<String>>>;

const KEEP_LINES: usize = 200;

/// Starts [cmd] hidden, with its stdout and stderr read into [output] (cleared first).
pub fn spawn(mut cmd: Command, output: &Output) -> std::io::Result<Child> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    #[cfg(windows)]
    job::attach(&child);

    output.lock().unwrap().clear();
    let pipes = [
        child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    ];
    for pipe in pipes.into_iter().flatten() {
        let out = output.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                let mut out = out.lock().unwrap();
                if out.len() >= KEEP_LINES {
                    out.pop_front();
                }
                out.push_back(line);
            }
        });
    }
    Ok(child)
}

pub fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub fn last_lines(output: &Output, n: usize) -> String {
    let out = output.lock().unwrap();
    out.iter().skip(out.len().saturating_sub(n)).cloned().collect::<Vec<_>>().join(" | ")
}

pub fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Physical cores are what ggml scales with; hyper-threads mostly get in the way.
/// Half the logical processors, capped at 8.
pub fn default_threads() -> u32 {
    let logical = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
    (logical / 2).clamp(2, 8)
}

#[cfg(windows)]
mod job {
    use std::process::Child;
    use std::sync::Mutex;

    use windows::Win32::Foundation::HANDLE;

    struct Job(HANDLE);
    // Only used behind the mutex below.
    unsafe impl Send for Job {}

    static JOB: Mutex<Option<Job>> = Mutex::new(None);

    fn create() -> Option<Job> {
        use windows::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .ok()?;
            Some(Job(job))
        }
    }

    /// The job is created once and never closed: when Amber's process ends, Windows closes
    /// the handle and kills every helper in it.
    pub fn attach(child: &Child) {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::System::JobObjects::AssignProcessToJobObject;
        let mut job = JOB.lock().unwrap();
        if job.is_none() {
            *job = create();
        }
        if let Some(job) = job.as_ref() {
            if let Err(e) = unsafe { AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle())) } {
                crate::diag::warn(format!("helper process not tied to the app: {e}"));
            }
        }
    }
}
