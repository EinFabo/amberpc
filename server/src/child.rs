//! Child processes (llama-server): no console window, output kept for the reason they die,
//! and ended with Amber Server - Linux: the parent-death signal; Windows: a kill-on-close job.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const KEEP_LINES: usize = 200;

pub struct Child {
    child: std::process::Child,
    output: Arc<Mutex<VecDeque<String>>>,
}

impl Child {
    pub fn spawn(mut cmd: Command) -> std::io::Result<Child> {
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            // Runs in the child between fork and exec: only the async-signal-safe prctl call.
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn()?;
        #[cfg(windows)]
        job::attach(&child);
        let output = Arc::new(Mutex::new(VecDeque::new()));
        let pipes: [Option<Box<dyn Read + Send>>; 2] = [
            child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
            child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
        ];
        for pipe in pipes.into_iter().flatten() {
            let out = output.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    let mut out = out.lock().unwrap_or_else(|e| e.into_inner());
                    if out.len() >= KEEP_LINES {
                        out.pop_front();
                    }
                    out.push_back(line);
                }
            });
        }
        Ok(Child { child, output })
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// The last lines it printed.
    pub fn output(&self) -> String {
        let out = self.output.lock().unwrap_or_else(|e| e.into_inner());
        out.iter().skip(out.len().saturating_sub(5)).cloned().collect::<Vec<_>>().join(" | ")
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.stop();
    }
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

    /// Created once and never closed: when Amber Server's process ends, Windows closes the
    /// handle and kills every child in it.
    pub fn attach(child: &Child) {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::System::JobObjects::AssignProcessToJobObject;
        let mut job = JOB.lock().unwrap_or_else(|e| e.into_inner());
        if job.is_none() {
            *job = create();
        }
        if let Some(job) = job.as_ref() {
            if let Err(e) = unsafe { AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle())) } {
                eprintln!("amber-server: a child process is not tied to the server: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_child_is_watched_and_its_output_kept() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "echo loading; echo failed >&2; exit 3"]);
        let mut child = Child::spawn(cmd).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while child.alive() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!child.alive());
        std::thread::sleep(std::time::Duration::from_millis(100));
        let out = child.output();
        assert!(out.contains("loading") && out.contains("failed"), "{out}");
    }
}
