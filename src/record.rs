use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::Msg;

pub const SYSTEM: &str = "default_output";
pub const MIC: &str = "default_input";

pub struct Recording {
    pid: Arc<AtomicI32>,
    stopped: Arc<AtomicBool>,
    pub path: PathBuf,
    pub size: (u32, u32),
}

pub struct Recorded {
    pub path: PathBuf,
    pub audio: Vec<&'static str>,
}

impl Recording {
    pub fn start(
        path: PathBuf,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        audio: Vec<&'static str>,
        tx: Sender<Msg>,
    ) -> Result<Self, String> {
        let region = format!("{w}x{h}+{x}+{y}");
        let child = spawn(&region, &audio, &path).map_err(|e| format!("gpu-screen-recorder: {e}"))?;
        let pid = Arc::new(AtomicI32::new(child.id() as i32));
        let stopped = Arc::new(AtomicBool::new(false));
        let (out, thread_pid, thread_stopped) = (path.clone(), pid.clone(), stopped.clone());
        std::thread::spawn(move || {
            let (mut child, mut audio) = (child, audio);
            loop {
                let last = last_error(&mut child);
                let status = child.wait();
                let written = out.metadata().is_ok_and(|m| m.len() > 0);
                let ok = status.as_ref().is_ok_and(|s| s.success()) && written;
                let mut error = format!("gpu-screen-recorder {status:?} {last}");
                if !ok && !written && !audio.is_empty() && !thread_stopped.load(Ordering::SeqCst) {
                    eprintln!("nino: recorder failed with audio ({last}), retrying without audio");
                    audio.clear();
                    match spawn(&region, &audio, &out) {
                        Ok(next) => {
                            thread_pid.store(next.id() as i32, Ordering::SeqCst);
                            child = next;
                            if thread_stopped.load(Ordering::SeqCst) {
                                signal(&thread_pid, libc::SIGINT);
                            }
                            continue;
                        }
                        Err(e) => error = format!("gpu-screen-recorder: {e}"),
                    }
                }
                let result = if ok { Ok(Recorded { path: out, audio }) } else { Err(error) };
                let _ = tx.send(Msg::RecordingDone(result));
                return;
            }
        });

        Ok(Self {
            pid,
            stopped,
            path,
            size: (w, h),
        })
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        signal(&self.pid, libc::SIGINT);
    }

    pub fn toggle_pause(&self) {
        signal(&self.pid, libc::SIGUSR2);
    }
}

fn spawn(region: &str, audio: &[&str], path: &Path) -> std::io::Result<Child> {
    Command::new("gpu-screen-recorder")
        .args(["-w", region])
        .args(["-f", "60", "-k", "h264", "-c", "mp4", "-cursor", "yes"])
        .args(audio.iter().flat_map(|a| ["-a", a]))
        .args(["-ac", "aac", "-fallback-cpu-encoding", "yes", "-o"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
}

fn last_error(child: &mut Child) -> String {
    use std::io::BufRead;
    let Some(stderr) = child.stderr.take() else {
        return String::new();
    };
    std::io::BufReader::new(stderr)
        .lines()
        .map_while(Result::ok)
        .filter(|l| l.contains("error"))
        .last()
        .unwrap_or_default()
}

fn signal(pid: &AtomicI32, sig: i32) {
    unsafe {
        libc::kill(pid.load(Ordering::SeqCst), sig);
    }
}

const RAW: &str = ".bruto.mp4";
const EXPORT_PREFIX: &str = ".export-";
const FINAL_STEM: &str = "gravacao";

pub fn raw_path(dir: &Path) -> PathBuf {
    dir.join(RAW)
}

pub fn publish(src: &Path, raw: &Path) -> std::io::Result<PathBuf> {
    for n in 1.. {
        let dst = raw.with_file_name(if n == 1 {
            format!("{FINAL_STEM}.mp4")
        } else {
            format!("{FINAL_STEM}-{n}.mp4")
        });
        match move_new(src, &dst) {
            Ok(()) => return Ok(dst),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn move_new(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(std::io::Error::other);
    let (from, to) = (c(src)?, c(dst)?);
    let moved = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if moved == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() != Some(libc::EINVAL) {
        return Err(err);
    }
    std::fs::hard_link(src, dst)?;
    std::fs::remove_file(src)
}

pub fn export_path(raw: &Path, id: u64) -> PathBuf {
    raw.with_file_name(format!("{EXPORT_PREFIX}{id}.mp4"))
}

fn flag(name: &str) -> bool {
    std::env::var_os("HOME").is_some_and(|home| PathBuf::from(home).join(".config/nino").join(name).exists())
}

pub fn wanted_audio() -> Vec<&'static str> {
    [(SYSTEM, "audio-pc"), (MIC, "microfone")]
        .into_iter()
        .filter(|&(_, name)| flag(name))
        .map(|(source, _)| source)
        .collect()
}

pub fn audio_devices() -> Vec<&'static str> {
    let listed = Command::new("gpu-screen-recorder")
        .arg("--list-audio-devices")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    [SYSTEM, MIC]
        .into_iter()
        .filter(|name| listed.lines().any(|l| l.split('|').next() == Some(name)))
        .collect()
}

pub fn recover(root: &Path) {
    let Ok(sessions) = std::fs::read_dir(root) else { return };
    for dir in sessions.flatten().map(|e| e.path()) {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(EXPORT_PREFIX) {
                let _ = std::fs::remove_file(&path);
            } else if name == RAW {
                match publish(&path, &path) {
                    Ok(to) => eprintln!("nino: recovered {}", to.display()),
                    Err(e) => eprintln!("nino: recover {}: {e}", path.display()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_never_replaces_an_existing_video() {
        let dir = std::env::temp_dir().join(format!("nino-publish-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let raw = raw_path(&dir);
        let export = export_path(&raw, 7);
        assert_eq!(export, dir.join(".export-7.mp4"));
        std::fs::write(dir.join("gravacao.mp4"), b"old").unwrap();
        std::fs::write(&export, b"new").unwrap();
        let out = publish(&export, &raw).unwrap();
        assert_eq!(out, dir.join("gravacao-2.mp4"));
        assert_eq!(std::fs::read(dir.join("gravacao.mp4")).unwrap(), b"old");
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
        assert!(!export.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
