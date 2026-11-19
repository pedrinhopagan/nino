use std::fmt::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::overlay::Doc;

pub type Region = (i32, i32, i32, i32);

pub struct Session {
    pub dir: PathBuf,
    pub region: Region,
    pub screen: (i32, i32),
    pub video: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub cancelled: bool,
    started: Instant,
    paused_total: Duration,
    paused_since: Option<Instant>,
}

pub struct Finished {
    pub files: Vec<PathBuf>,
    pub text: Option<String>,
}

pub struct Entry {
    pub at: Duration,
    pub text: String,
    pub png: Option<Vec<u8>>,
    pub doc: Option<Doc>,
}

impl Session {
    pub fn new(region: Region, screen: (i32, i32)) -> Result<Self, String> {
        let name = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let dir = std::env::temp_dir().join("nino").join(name);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Self {
            dir,
            region,
            screen,
            video: None,
            entries: Vec::new(),
            cancelled: false,
            started: Instant::now(),
            paused_total: Duration::ZERO,
            paused_since: None,
        })
    }

    pub fn video_path(&self) -> PathBuf {
        self.dir.join("gravacao.mp4")
    }

    pub fn elapsed(&self) -> Duration {
        let paused = self.paused_total + self.paused_since.map_or(Duration::ZERO, |t| t.elapsed());
        self.started.elapsed().saturating_sub(paused)
    }

    pub fn pause(&mut self) -> bool {
        if self.paused_since.is_some() {
            return false;
        }
        self.paused_since = Some(Instant::now());
        true
    }

    pub fn resume(&mut self) -> bool {
        let Some(since) = self.paused_since.take() else {
            return false;
        };
        self.paused_total += since.elapsed();
        true
    }

    pub fn editor_state(&self, at: Duration) -> String {
        let entries: Vec<String> = self
            .entries
            .iter()
            .map(|e| {
                format!(
                    "{{\"clock\":{},\"text\":{},\"image\":{}}}",
                    json(&clock(e.at)),
                    json(&e.text),
                    e.png.is_some()
                )
            })
            .collect();
        format!("{{\"clock\":{},\"entries\":[{}]}}", json(&clock(at)), entries.join(","))
    }

    pub fn finish(self) -> Result<Finished, String> {
        let mut files: Vec<PathBuf> = self.video.iter().cloned().collect();
        let mut out = String::new();
        if let Some(video) = &self.video {
            let _ = writeln!(out, "Vídeo: {}\n", video.display());
        }
        for (i, entry) in self.entries.iter().enumerate() {
            let _ = write!(out, "Anotação {} - {}", i + 1, clock(entry.at));
            if let Some(png) = &entry.png {
                let path = self.dir.join(format!("anotacao-{}.png", i + 1));
                std::fs::write(&path, png).map_err(|e| format!("{}: {e}", path.display()))?;
                let _ = write!(out, " ({})", path.display());
                files.push(path);
            }
            if entry.text.is_empty() {
                out.push('\n');
            } else {
                let _ = writeln!(out, ": \"{}\"", entry.text);
            }
        }
        let text = self.entries.iter().any(|e| !e.text.is_empty()).then_some(out);
        Ok(Finished { files, text })
    }
}

fn clock(at: Duration) -> String {
    let s = at.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c < ' ' || matches!(c, '<' | '\u{2028}' | '\u{2029}') => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
