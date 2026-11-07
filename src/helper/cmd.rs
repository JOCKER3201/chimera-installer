// Uruchamianie poleceń w helperze roota ze strumieniowaniem wyjścia
// do GUI liniowym protokołem na stdout:
//   ::P <krok> <razem> <opis>   — postęp
//   ::L <tekst>                 — linia logu
//   ::D                         — sukces
//   ::E <komunikat>             — błąd
// Wszystko inne GUI traktuje jak zwykły log.

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
    // stdout i stderr dziecka płyną do logu na bieżąco, każdy z osobnego wątku.
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
        .map_err(|e| format!("oczekiwanie na `{display}` nie powiodło się: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{display}` zakończyło się kodem {status}"))
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
        .map_err(|e| format!("nie można uruchomić `{display}`: {e}"))?;
    stream_child(child, display)
}

/// Uruchamia polecenie, strumieniując całe wyjście do logu.
pub fn run(args: &[&str]) -> Result<(), String> {
    run_with_display(args, &display_of(args))
}

/// Jak `run`, ale podaje `input` na stdin (sfdisk, chpasswd).
/// `secret` wyłącza logowanie treści wejścia.
pub fn run_stdin(args: &[&str], input: &str, secret: bool) -> Result<(), String> {
    let display = display_of(args);
    if secret {
        log(&format!("$ {display}  (dane wejściowe ukryte)"));
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
        .map_err(|e| format!("nie można uruchomić `{display}`: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or("brak stdin procesu potomnego")?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("zapis na stdin `{display}` nie powiódł się: {e}"))?;
    }
    stream_child(child, &display)
}

/// Uruchamia polecenie i zwraca jego stdout (stderr idzie do logu).
pub fn run_capture(args: &[&str]) -> Result<String, String> {
    let display = display_of(args);
    log(&format!("$ {display}"));
    let out = Command::new(args[0])
        .args(&args[1..])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("nie można uruchomić `{display}`: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        log(stderr.trim());
    }
    if !out.status.success() {
        return Err(format!("`{display}` zakończyło się kodem {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Uruchamia polecenie wewnątrz chroota `root`.
pub fn run_in(root: &Path, args: &[&str]) -> Result<(), String> {
    let root_s = root.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["chroot", &root_s];
    full.extend_from_slice(args);
    run(&full)
}

/// Uruchamia polecenie powłoki jako wskazany użytkownik (zrzut uprawnień
/// przez uid/gid z biblioteki standardowej — bez zależności od `setpriv`,
/// którego nośnik live Chimery nie zawiera). Czyste środowisko z HOME/PATH.
pub fn run_as_user(
    uid: u32,
    gid: u32,
    user: &str,
    home: &str,
    cwd: &str,
    shell_cmd: &str,
) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    log(&format!("$ (jako {user}, w {cwd}) {shell_cmd}"));
    let child = Command::new("sh")
        .arg("-c")
        .arg(shell_cmd)
        .uid(uid)
        .gid(gid)
        // Katalog roboczy musi należeć do użytkownika — bez tego proces
        // dziedziczy cwd roota (niedostępne), a `.` w poleceniach celuje tam.
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
        .map_err(|e| format!("nie można uruchomić polecenia jako {user}: {e}"))?;
    stream_child(child, shell_cmd)
}

/// Jak `run_in`, ale argument o indeksie `hide` (licząc w `args`) jest
/// w logu zastąpiony znacznikiem — do przekazywania hashy haseł.
pub fn run_in_redacted(root: &Path, args: &[&str], hide: usize) -> Result<(), String> {
    let root_s = root.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["chroot", &root_s];
    full.extend_from_slice(args);
    let display = full
        .iter()
        .enumerate()
        .map(|(i, a)| if i == hide + 2 { "(ukryte)" } else { a })
        .collect::<Vec<_>>()
        .join(" ");
    run_with_display(&full, &display)
}

/// Uruchamia polecenie w chroocie z wejściem na stdin i zwraca jego stdout.
/// Treść wejścia nie trafia do logu (hasła).
pub fn run_in_capture_stdin(root: &Path, args: &[&str], input: &str) -> Result<String, String> {
    let root_s = root.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["chroot", &root_s];
    full.extend_from_slice(args);
    let display = display_of(&full);
    log(&format!("$ {display}  (dane wejściowe ukryte)"));
    let mut child = Command::new(full[0])
        .args(&full[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("nie można uruchomić `{display}`: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or("brak stdin procesu potomnego")?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("zapis na stdin `{display}` nie powiódł się: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("oczekiwanie na `{display}` nie powiodło się: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        log(stderr.trim());
    }
    if !out.status.success() {
        return Err(format!("`{display}` zakończyło się kodem {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Wariant `run`, którego niepowodzenie jest tylko ostrzeżeniem w logu.
pub fn run_soft(args: &[&str]) {
    if let Err(e) = run(args) {
        log(&format!("OSTRZEŻENIE (ignoruję): {e}"));
    }
}

pub fn run_in_soft(root: &Path, args: &[&str]) {
    let root_s = root.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["chroot", &root_s];
    full.extend_from_slice(args);
    run_soft(&full);
}
