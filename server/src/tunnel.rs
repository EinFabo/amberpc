//! The API from anywhere: a Cloudflare Tunnel, run by cloudflared with the owner's token - as
//! Amber for Windows does. The owner creates the tunnel in Cloudflare (a public hostname on
//! their domain pointing at http://localhost:<port>) and hands its token to
//! `amber-server tunnel setup`; the token is kept in a file only the owner can read and
//! reaches cloudflared in its environment, never on its command line.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// After cloudflared ended on its own: the next try.
const RETRY: Duration = Duration::from_secs(30);

/// Where the token is kept: beside the config, readable by its owner only.
pub fn token_path() -> PathBuf {
    crate::config::Config::path().with_file_name("tunnel-token")
}

pub fn load_token(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// Saves [token] - private from the first byte, replaced in one step.
pub fn save_token(path: &Path, token: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("no folder for the token")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let temp = path.with_extension("new");
    let _ = std::fs::remove_file(&temp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temp).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, token.trim().as_bytes()).map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&temp, path).map_err(|e| e.to_string())
}

/// cloudflared for this machine, downloaded (pinned, checked) if it is not there yet.
pub fn ensure_cloudflared(data: &Path) -> Result<PathBuf, String> {
    let os = if cfg!(windows) { "windows" } else { "linux" };
    let tool = amber_core::local::cloudflared_for(os).ok_or("no cloudflared for this system")?;
    let exe = data.join("tools").join(format!("cloudflared-{}{}", tool.version, if cfg!(windows) { ".exe" } else { "" }));
    if exe.is_file() {
        return Ok(exe);
    }
    crate::catalog::download(tool.url, tool.sha256, &exe, "cloudflared")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(exe)
}

/// --url: where a tunnel made on the command line sends its requests (127.0.0.1, not
/// "localhost", which may be another program on ::1); one set up in Cloudflare's dashboard
/// brings its own routing. The token in the environment.
pub fn cloudflared_command(exe: &Path, port: u16, token: &str) -> Command {
    let mut cmd = Command::new(exe);
    cmd.args(["tunnel", "--no-autoupdate", "--url", &format!("http://127.0.0.1:{port}"), "run"]).env("TUNNEL_TOKEN", token);
    cmd
}

/// Whether cloudflared said it is through to Cloudflare.
pub fn connected(lines: &[String]) -> bool {
    lines.iter().any(|l| l.contains("Registered tunnel connection"))
}

/// What to tell when cloudflared ended, from its last words.
pub fn why_it_ended(last: &str) -> String {
    if last.contains("token is not valid") || last.contains("Invalid tunnel secret") {
        "the tunnel token is not valid - copy it again from Cloudflare: amber-server tunnel setup <hostname>".into()
    } else {
        format!("cloudflared ended: {last}")
    }
}

/// Keeps cloudflared running for as long as the server does: started (downloaded first if
/// needed), watched, and started again a little after it ends.
pub fn keep_running(port: u16, data: PathBuf, log: fn(String)) {
    std::thread::spawn(move || loop {
        let started = (|| {
            let token = load_token(&token_path()).ok_or("no tunnel token - amber-server tunnel setup <hostname>")?;
            let exe = ensure_cloudflared(&data)?;
            crate::child::Child::spawn(cloudflared_command(&exe, port, &token)).map_err(|e| format!("cloudflared could not start: {e}"))
        })();
        match started {
            Ok(mut child) => {
                log("tunnel starting".into());
                let mut said = false;
                while child.alive() {
                    if !said && connected(&child.output().split(" | ").map(str::to_string).collect::<Vec<_>>()) {
                        log("tunnel connected".into());
                        said = true;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                log(format!("tunnel: {}", why_it_ended(&child.output())));
            }
            Err(e) => log(format!("tunnel: {e}")),
        }
        std::thread::sleep(RETRY);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_token_stays_off_the_command_line() {
        let cmd = cloudflared_command(Path::new("/x/cloudflared"), 8766, "eyJhIjoi.secret");
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["tunnel", "--no-autoupdate", "--url", "http://127.0.0.1:8766", "run"]);
        let envs: Vec<(String, Option<String>)> =
            cmd.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect();
        assert!(envs.contains(&("TUNNEL_TOKEN".into(), Some("eyJhIjoi.secret".into()))));
    }

    #[test]
    fn the_token_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tunnel-token");
        assert_eq!(load_token(&path), None);
        save_token(&path, "eyJhIjoi.secret").unwrap();
        assert_eq!(load_token(&path).as_deref(), Some("eyJhIjoi.secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn cloudflareds_last_words_are_told_plainly() {
        assert!(why_it_ended("ERR Provided Tunnel token is not valid").contains("copy it again"));
        assert!(why_it_ended("INF Initiating graceful shutdown").contains("cloudflared ended"));
        assert!(connected(&["INF Registered tunnel connection connIndex=0".to_string()]));
        assert!(!connected(&["INF Starting tunnel".to_string()]));
    }
}
