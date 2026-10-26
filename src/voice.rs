//! Narration audio per shot, and the timing it implies.
//!
//! Source priority for each shot:
//!   1. a recording you made: videos/<id>/vo/<shot>.{wav,mp3,m4a,flac}   <- best for the channel
//!   2. piper TTS, if configured in forge.toml
//!   3. silence timed from the word count, so you can cut the picture before recording

use crate::config::{Project, Shot, Video};
use crate::util::{self, ffmpeg, FNV_SEED};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShotTiming {
    pub id: String,
    pub source: String,
    pub voice_seconds: f64,
    pub start: f64,
    pub seconds: f64,
    pub frames: u32,
}

pub fn audio_dir(p: &Project, id: &str) -> PathBuf {
    p.root.join("build").join(id).join("audio")
}

fn recorded(p: &Project, id: &str, shot: &str) -> Option<PathBuf> {
    ["wav", "mp3", "m4a", "flac"]
        .iter()
        .map(|ext| p.video_dir(id).join("vo").join(format!("{shot}.{ext}")))
        .find(|f| f.exists())
}

/// Make sure audio/<shot>.wav exists and is current. Returns (source label, seconds of speech).
fn ensure_shot_audio(p: &Project, id: &str, shot: &Shot) -> Result<(String, f64)> {
    let dir = audio_dir(p, id);
    fs::create_dir_all(&dir)?;
    let wav = dir.join(format!("{}.wav", shot.id));
    let stamp = dir.join(format!("{}.key", shot.id));
    let v = &p.forge.voice;

    let (source, key) = if let Some(rec) = recorded(p, id, &shot.id) {
        let m = fs::metadata(&rec)?.modified()?;
        ("recorded".to_string(), format!("rec:{}:{:?}", rec.display(), m))
    } else if !v.piper.is_empty() && !v.piper_model.is_empty() {
        ("piper".to_string(), format!("piper:{}:{}", v.piper_model, shot.narration))
    } else {
        ("silent".to_string(), format!("silent:{}:{}", v.words_per_minute, shot.narration))
    };
    let key = format!("{:016x}", util::fnv1a(key.as_bytes(), FNV_SEED));

    if wav.exists() && fs::read_to_string(&stamp).ok().as_deref() == Some(key.as_str()) {
        let secs = util::duration(&wav)?;
        return Ok((source, secs));
    }

    match source.as_str() {
        "recorded" => {
            let rec = recorded(p, id, &shot.id).unwrap();
            // Light cleanup for a home recording: high-pass rumble, trim leading/trailing silence.
            util::run(ffmpeg().arg("-i").arg(&rec).args([
                "-af",
                "highpass=f=80,silenceremove=start_periods=1:start_threshold=-45dB,areverse,silenceremove=start_periods=1:start_threshold=-45dB,areverse",
                "-ar", "48000", "-ac", "1",
            ]).arg(&wav))?;
        }
        "piper" => {
            let raw = dir.join(format!("{}.piper.wav", shot.id));
            let mut child = Command::new(&v.piper)
                .args(["--model", &v.piper_model, "--output_file"])
                .arg(&raw)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .with_context(|| format!("starting piper at '{}'", v.piper))?;
            child.stdin.take().unwrap().write_all(shot.narration.as_bytes())?;
            let out = child.wait_with_output()?;
            anyhow::ensure!(out.status.success(), "piper failed: {}", String::from_utf8_lossy(&out.stderr));
            util::run(ffmpeg().arg("-i").arg(&raw).args(["-ar", "48000", "-ac", "1"]).arg(&wav))?;
            let _ = fs::remove_file(raw);
        }
        _ => {
            let secs = (util::word_count(&shot.narration) as f64 / v.words_per_minute * 60.0).max(0.5);
            util::run(ffmpeg().args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono", "-t", &format!("{secs:.3}")]).arg(&wav))?;
        }
    }
    fs::write(&stamp, &key)?;
    Ok((source, util::duration(&wav)?))
}

/// Produce narration audio for every shot and the per-shot timing for a given fps.
pub fn prepare(p: &Project, id: &str, video: &Video, fps: u32) -> Result<Vec<ShotTiming>> {
    let mut out = Vec::new();
    let mut start = 0.0;
    for shot in &video.shots {
        let (source, voice_seconds) = ensure_shot_audio(p, id, shot)?;
        let seconds = (voice_seconds + p.forge.voice.lead_in + shot.pad_seconds).max(shot.min_seconds);
        let frames = (seconds * fps as f64).ceil() as u32;
        let seconds = frames as f64 / fps as f64;
        out.push(ShotTiming { id: shot.id.clone(), source, voice_seconds, start, seconds, frames });
        start += seconds;
    }
    Ok(out)
}

pub fn save_timing(path: &Path, t: &[ShotTiming]) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(t)?)?;
    Ok(())
}
