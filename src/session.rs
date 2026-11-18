use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct Session {
    pub dir: PathBuf,
    started: Instant,
    pub editing: bool,
    pub video: Option<PathBuf>,
    entries: Vec<Entry>,
}

struct Entry {
    at: Duration,
    print: Option<String>,
    text: String,
}

pub struct Finished {
    pub files: Vec<PathBuf>,
    pub notes: Option<String>,
}

impl Session {
    pub fn new() -> Result<Self, String> {
        let name = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let dir = videos_dir().join("Nino").join(name);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Self {
            dir,
            started: Instant::now(),
            editing: false,
            video: None,
            entries: Vec::new(),
        })
    }

    pub fn video_path(&self) -> PathBuf {
        self.dir.join("gravacao.mp4")
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn add_note(&mut self, at: Duration) -> (usize, String) {
        let n = self
            .entries
            .iter()
            .filter(|e| e.print.is_none() && !e.text.is_empty())
            .count()
            + 1;
        self.entries.push(Entry {
            at,
            print: None,
            text: String::new(),
        });
        (self.entries.len() - 1, format!("Anotação {n} · {}", clock(at)))
    }

    pub fn add_print(&mut self, at: Duration, png: &[u8]) -> Result<(usize, String), String> {
        let n = self.entries.iter().filter(|e| e.print.is_some()).count() + 1;
        let name = format!("print-{n}.png");
        let path = self.dir.join(&name);
        std::fs::write(&path, png).map_err(|e| format!("{}: {e}", path.display()))?;
        self.entries.push(Entry {
            at,
            print: Some(name),
            text: String::new(),
        });
        Ok((self.entries.len() - 1, format!("Print {n} · {}", clock(at))))
    }

    pub fn set_text(&mut self, id: usize, text: String) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.text = text;
        }
    }

    pub fn finish(self) -> Result<Finished, String> {
        let entries: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.print.is_some() || !e.text.is_empty())
            .collect();
        let mut files: Vec<PathBuf> = self.video.iter().cloned().collect();
        if entries.is_empty() {
            return Ok(Finished { files, notes: None });
        }
        let notes = render(&self.dir, self.video.as_deref(), &entries);
        let md = self.dir.join("anotacoes.md");
        std::fs::write(&md, &notes).map_err(|e| format!("{}: {e}", md.display()))?;
        files.extend(
            entries
                .iter()
                .filter_map(|e| e.print.as_ref().map(|p| self.dir.join(p))),
        );
        files.push(md);
        Ok(Finished {
            files,
            notes: Some(notes),
        })
    }
}

fn render(dir: &Path, video: Option<&Path>, entries: &[&Entry]) -> String {
    let mut out = format!("Pasta: {}\n", dir.display());
    if let Some(name) = video.and_then(Path::file_name) {
        let _ = writeln!(out, "Vídeo: {}", name.to_string_lossy());
    }
    out.push('\n');
    let (mut notes, mut prints) = (0, 0);
    for entry in entries {
        let label = match &entry.print {
            Some(file) => {
                prints += 1;
                format!("Print {prints} - {} ({file})", clock(entry.at))
            }
            None => {
                notes += 1;
                format!("Anotação {notes} - {}", clock(entry.at))
            }
        };
        if entry.text.is_empty() {
            let _ = writeln!(out, "{label}");
        } else {
            let _ = writeln!(out, "{label}: \"{}\"", entry.text);
        }
    }
    out
}

fn clock(at: Duration) -> String {
    let s = at.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

fn videos_dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let dirs = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    dirs.lines()
        .find_map(|l| l.strip_prefix("XDG_VIDEOS_DIR="))
        .map(|v| v.trim_matches('"').replace("$HOME", &home.to_string_lossy()))
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("Videos"))
}
