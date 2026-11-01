//! Child processes (llama-server): no console window, output kept for the reason they die,
//! and ended with Amber Server - Linux: the parent-death signal; Windows: a kill-on-close job.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};

const KEEP_LINES: usize = 200;

type Job = (Command, mpsc::Sender<std::io::Result<std::process::Child>>);

/// Every child is started from this one thread, which lives as long as the server: Linux's
/// parent-death signal follows the thread that started a child, not the process - started
/// from an HTTP request's thread, llama-server died after every request.
fn spawner() -> &'static Mutex<mpsc::Sender<Job>> {
    static SPAWNER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    SPAWNER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("amber-spawner".into())
            .spawn(move || {
                for (mut cmd, reply) in rx {
                    let _ = reply.send(cmd.spawn());
                }
            })
            .expect("the spawner thread");
        Mutex::new(tx)
    })
}

pub struct Child {
    child: std::process::Child,
    /// stdout and stderr apart: llama.cpp says why it failed on stderr, and its stdout must
    /// not push that out of the last lines.
    out: Arc<Mutex<VecDeque<String>>>,
    err: Arc<Mutex<VecDeque<String>>>,
    /// One message per reader that reached the end of its pipe.
    done: mpsc::Receiver<()>,
    readers: usize,
}

fn keep(buffer: Arc<Mutex<VecDeque<String>>>, pipe: impl Read + Send + 'static, done: mpsc::Sender<()>) {
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            let mut b = buffer.lock().unwrap_or_else(|e| e.into_inner());
            if b.len() >= KEEP_LINES {
                b.pop_front();
            }
            b.push_back(line);
        }
        let _ = done.send(());
    });
}

fn last(buffer: &Mutex<VecDeque<String>>, n: usize) -> Vec<String> {
    let b = buffer.lock().unwrap_or_else(|e| e.into_inner());
    b.iter().skip(b.len().saturating_sub(n)).cloned().collect()
}

impl Child {
    pub fn spawn(mut cmd: Command) -> std::io::Result<Child> {
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            let parent = std::process::id() as libc::pid_t;
            // Runs in the child between fork and exec: only async-signal-safe calls. A parent
            // that died before prctl took effect would never send the signal - so stop here.
            cmd.pre_exec(move || {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                if libc::getppid() != parent {
                    return Err(std::io::Error::other("the server ended while starting this"));
                }
                Ok(())
            });
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let (tx, rx) = mpsc::channel();
        spawner().lock().unwrap_or_else(|e| e.into_inner()).send((cmd, tx)).map_err(|_| std::io::Error::other("the spawner thread is gone"))?;
        let mut child = rx.recv().map_err(|_| std::io::Error::other("the spawner thread is gone"))??;
        #[cfg(windows)]
        job::attach(&child);
        let (out, err) = (Arc::new(Mutex::new(VecDeque::new())), Arc::new(Mutex::new(VecDeque::new())));
        let (tx, done) = mpsc::channel();
        let mut readers = 0;
        if let Some(pipe) = child.stdout.take() {
            keep(out.clone(), pipe, tx.clone());
            readers += 1;
        }
        if let Some(pipe) = child.stderr.take() {
            keep(err.clone(), pipe, tx);
            readers += 1;
        }
        Ok(Child { child, out, err, done, readers })
    }

    pub fn alive(&mut self) -> bool {
        let running = matches!(self.child.try_wait(), Ok(None));
        if !running {
            // Ended: the readers finish with what is left - unless something it started holds
            // the pipe open, so not longer than a second.
            let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while self.readers > 0 {
                let left = until.saturating_duration_since(std::time::Instant::now());
                if self.done.recv_timeout(left).is_err() {
                    self.readers = 0;
                    break;
                }
                self.readers -= 1;
            }
        }
        running
    }

    /// Whether any line still kept (the last 200 of each stream) contains [needle].
    pub fn saw(&self, needle: &str) -> bool {
        [&self.out, &self.err].iter().any(|b| b.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|l| l.contains(needle)))
    }

    /// The last lines it printed - on stderr if any, the reason when it dies.
    pub fn output(&self) -> String {
        let err = last(&self.err, 5);
        if err.is_empty() { last(&self.out, 5) } else { err }.join(" | ")
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
    fn a_line_is_found_among_all_kept() {
        // cloudflared says it connected, then prints on: the last five lines miss it.
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "echo 'INF Registered tunnel connection' >&2; for i in 1 2 3 4 5 6 7 8; do echo more$i >&2; done"]);
        let mut child = Child::spawn(cmd).unwrap();
        while child.alive() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!child.output().contains("Registered"));
        assert!(child.saw("Registered tunnel connection"));
        assert!(!child.saw("nothing like this"));
    }

    #[cfg(unix)]
    #[test]
    fn a_grandchild_holding_the_pipe_does_not_hang_the_check() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sleep 5 & echo started; exit 0"]);
        let mut child = Child::spawn(cmd).unwrap();
        let t = std::time::Instant::now();
        while child.alive() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(t.elapsed() < std::time::Duration::from_secs(3), "waited for the grandchild: {:?}", t.elapsed());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_child_outlives_the_thread_that_started_it() {
        // Review finding: the parent-death signal follows the spawning thread, and the HTTP
        // server's threads end after every request.
        let child = std::thread::spawn(|| {
            let mut cmd = std::process::Command::new("sleep");
            cmd.arg("5");
            Child::spawn(cmd).unwrap()
        })
        .join()
        .unwrap();
        let mut child = child;
        std::thread::sleep(std::time::Duration::from_millis(600));
        assert!(child.alive(), "killed when its starting thread ended");
        child.stop();
    }

    #[cfg(unix)]
    #[test]
    fn a_child_is_watched_and_its_output_kept() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "echo loading; sleep 0.2; seq -f line%g 1 20000; echo failed >&2; exit 3"]);
        let mut child = Child::spawn(cmd).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while child.alive() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!child.alive());
        // No waiting: once it has ended, everything it printed is there.
        let out = child.output();
        assert!(out.contains("failed") && !out.contains("line"), "the reason, not drowned by stdout: {out}");
    }
}
