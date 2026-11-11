// Running commands in the root helper, streaming their output to the GUI
// over a line protocol on stdout:
//   ::P <step> <total> <description>   — progress
//   ::L <text>                         — log line
//   ::D                                — success
//   ::E <message>                      — error
// The GUI treats everything else as a plain log line.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

pub fn log(text: &str) {
    for line in text.lines() {
        println!("::L {line}");
    }
    let _ = std::io::stdout().flush();
}

pub fn progress(step: u32, total: u32, text: &str) {
    println!("::P {step} {total} {text}");
    let _ = std::io::stdout().flush();
}

pub fn done() {
    println!("::D");
    let _ = std::io::stdout().flush();
}

pub fn fail(msg: &str) {
    println!("::E {}", msg.replace('\n', " | "));
    let _ = std::io::stdout().flush();
}

fn stream_child(mut child: std::process::Child, display: &str) -> Result<(), String> {
    // The child's stdout and stderr stream into the log live, each from its
    // own thread.
    let out = child.stdout.take();
    let err = child.stderr.take();
    let mut readers = Vec::new();
    if let Some(o) = out {
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(o).lines().map_while(Result::ok) {
                log(&line);
            }
        }));
    }
    if let Some(e) = err {
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(e).lines().map_while(Result::ok) {
                log(&line);
            }
        }));
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child
        .wait()
        .map_err(|e| format!("waiting for `{display}` failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{display}` exited with status {status}"))
    }
}

fn display_of(args: &[&str]) -> String {
    args.join(" ")
}

fn run_with_display(args: &[&str], display: &str) -> Result<(), String> {
    log(&format!("$ {display}"));
    let child = Command::new(args[0])
        .args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run `{display}`: {e}"))?;
    stream_child(child, display)
}

/// Runs a command, streaming all of its output to the log.
pub fn run(args: &[&str]) -> Result<(), String> {
    run_with_display(args, &display_of(args))
}

/// Like `run`, but feeds `input` on stdin (sfdisk, chpasswd).
/// `secret` disables logging of the input contents.
pub fn run_stdin(args: &[&str], input: &str, secret: bool) -> Result<(), String> {
    let display = display_of(args);
    if secret {
        log(&format!("$ {display}  (input hidden)"));
    } else {
        log(&format!("$ {display} <<EOF"));
        log(input);
        log("EOF");
    }
    let mut child = Command::new(args[0])
        .args(&args[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run `{display}`: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or("child process has no stdin")?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("writing to the stdin of `{display}` failed: {e}"))?;
    }
    stream_child(child, &display)
}

/// Runs a command and returns its stdout (stderr goes to the log).
pub fn run_capture(args: &[&str]) -> Result<String, String> {
    let display = display_of(args);
    log(&format!("$ {display}"));
    let out = Command::new(args[0])
        .args(&args[1..])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run `{display}`: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        log(stderr.trim());
    }
    if !out.status.success() {
        return Err(format!("`{display}` exited with status {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Runs a command inside the `root` chroot.
pub fn run_in(root: &Path, args: &[&str]) -> Result<(), String> {
    let root_s = root.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["chroot", &root_s];
    full.extend_from_slice(args);
    run(&full)
}

/// Runs a shell command as the given user (privileges dropped via uid/gid
/// from the standard library — no dependency on `setpriv`, which Chimera's
/// live medium does not ship). Clean environment with HOME/PATH.
pub fn run_as_user(
    uid: u32,
    gid: u32,
    user: &str,
    home: &str,
    cwd: &str,
    shell_cmd: &str,
) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    log(&format!("$ (as {user}, in {cwd}) {shell_cmd}"));
    let child = Command::new("sh")
        .arg("-c")
        .arg(shell_cmd)
        .uid(uid)
        .gid(gid)
        // The working directory must belong to the user — otherwise the
        // process inherits root's cwd (inaccessible), and `.` in commands
        // points there.
        .current_dir(cwd)
        .env_clear()
        .env("HOME", home)
        .env("USER", user)
        .env("LOGNAME", user)
        .env("PATH", "/usr/bin:/usr/sbin:/bin:/sbin")
        .env("TMPDIR", cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run a command as {user}: {e}"))?;
    stream_child(child, shell_cmd)
}

/// Like `run_in`, but the argument at index `hide` (counted within `args`)
/// is replaced with a marker in the log — for passing password hashes.
pub fn run_in_redacted(root: &Path, args: &[&str], hide: usize) -> Result<(), String> {
    let root_s = root.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["chroot", &root_s];
    full.extend_from_slice(args);
    let display = full
        .iter()
        .enumerate()
        .map(|(i, a)| if i == hide + 2 { "(hidden)" } else { a })
        .collect::<Vec<_>>()
        .join(" ");
    run_with_display(&full, &display)
}

/// Runs a command in the chroot with input on stdin and returns its stdout.
/// The input contents never reach the log (passwords).
pub fn run_in_capture_stdin(root: &Path, args: &[&str], input: &str) -> Result<String, String> {
    let root_s = root.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["chroot", &root_s];
    full.extend_from_slice(args);
    let display = display_of(&full);
    log(&format!("$ {display}  (input hidden)"));
    let mut child = Command::new(full[0])
        .args(&full[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run `{display}`: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or("child process has no stdin")?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("writing to the stdin of `{display}` failed: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("waiting for `{display}` failed: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        log(stderr.trim());
    }
    if !out.status.success() {
        return Err(format!("`{display}` exited with status {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// A `run` variant whose failure is only a warning in the log.
pub fn run_soft(args: &[&str]) {
    if let Err(e) = run(args) {
        log(&format!("WARNING (ignoring): {e}"));
    }
}

pub fn run_in_soft(root: &Path, args: &[&str]) {
    let root_s = root.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["chroot", &root_s];
    full.extend_from_slice(args);
    run_soft(&full);
}
