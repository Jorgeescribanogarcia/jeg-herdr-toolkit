//! Running a command with a time limit: a hook must never hang on a network call.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const GIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Like `Command::output`, but the process is killed if it has not finished within `limit`. Both pipes
/// are read in threads while waiting, so a command with a lot of output cannot block on a full pipe.
pub fn output_within(mut command: Command, limit: Duration) -> Result<Output, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {:?}: {e}", command.get_program()))?;
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                // A read error only loses output; the exit status still says how it went.
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let stderr = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() >= limit {
            // Already exiting or gone if this fails; either way it is not waited for any longer.
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{:?} did not finish in {} s", command.get_program(), limit.as_secs()));
        }
        thread::sleep(Duration::from_millis(50));
    };
    Ok(Output { status, stdout: stdout.join().unwrap_or_default(), stderr: stderr.join().unwrap_or_default() })
}

/// `git ls-remote` of a GitHub repo, never interactive: a repo that was deleted or made private answers
/// with a login challenge, and a background hook must not open a sign-in window or wait for a password.
/// `repo` must already be a checked `owner/repo`.
pub fn git_ls_remote(repo: &str) -> Result<String, String> {
    let mut git = Command::new("git");
    git.env("GIT_TERMINAL_PROMPT", "0").env("GCM_INTERACTIVE", "never").args([
        "-c",
        "credential.helper=",
        "ls-remote",
        "--",
        &format!("https://github.com/{repo}"),
    ]);
    let output = output_within(git, GIT_TIMEOUT)?;
    if !output.status.success() {
        return Err(format!("git ls-remote: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn slow_command_is_killed() {
        let started = Instant::now();
        let mut slow = Command::new("cmd");
        slow.args(["/c", "ping -n 6 127.0.0.1 >nul"]);
        let error = output_within(slow, Duration::from_millis(300)).unwrap_err();
        assert!(error.contains("did not finish"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn fast_command_gives_its_output() {
        let mut quick = Command::new("cmd");
        quick.args(["/c", "echo hello"]);
        let out = output_within(quick, Duration::from_secs(10)).unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }
}
