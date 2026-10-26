use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};

/// Run a command, failing with its stderr tail if it exits non-zero.
pub fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("starting {:?}", cmd.get_program()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(15).collect();
        bail!("{:?} failed:\n{}", cmd.get_program(), tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}

pub fn ffmpeg() -> Command {
    let mut c = Command::new("ffmpeg");
    c.args(["-y", "-hide_banner", "-loglevel", "error"]);
    c
}

/// Duration of a media file in seconds.
pub fn duration(path: &Path) -> Result<f64> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .context("running ffprobe")?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim().parse::<f64>().with_context(|| format!("no duration for {}", path.display()))
}

/// FNV-1a: tiny, stable across Rust versions (unlike DefaultHasher), good enough for cache keys.
pub fn fnv1a(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub const FNV_SEED: u64 = 0xcbf29ce484222325;

pub fn srt_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000)
}

pub fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}
