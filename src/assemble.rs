//! Frames + narration -> clips -> one video, plus captions, chapters, thumbnail and metadata.

use crate::config::{Profile, Project, Video};
use crate::util::{self, ffmpeg, srt_time};
use crate::voice::{audio_dir, ShotTiming};
use anyhow::{Context, Result};
use serde_json::json;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub fn clips(p: &Project, id: &str, prof: &Profile, timing: &[ShotTiming], frames_root: &Path, clips_dir: &Path) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(clips_dir)?;
    let delay_ms = (p.forge.voice.lead_in * 1000.0).round() as u64;
    let mut out = Vec::new();
    for t in timing {
        let clip = clips_dir.join(format!("{}.mp4", t.id));
        let frames = frames_root.join(&t.id).join("f_%04d.png");
        let wav = audio_dir(p, id).join(format!("{}.wav", t.id));
        util::run(
            ffmpeg()
                .args(["-framerate", &prof.fps.to_string(), "-i"]).arg(&frames)
                .arg("-i").arg(&wav)
                .args(["-filter_complex", &format!("[1:a]adelay={delay_ms}:all=1,apad[a]")])
                .args(["-map", "0:v", "-map", "[a]", "-frames:v", &t.frames.to_string()])
                .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p"])
                .args(["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-shortest"])
                .arg(&clip),
        )
        .with_context(|| format!("encoding clip {}", t.id))?;
        out.push(clip);
    }
    Ok(out)
}

/// Concatenate clips, lay the music bed under the voice (ducked), master to YouTube loudness.
pub fn master(p: &Project, clips: &[PathBuf], work: &Path, out: &Path) -> Result<()> {
    let list = work.join("concat.txt");
    let mut s = String::new();
    for c in clips {
        writeln!(s, "file '{}'", c.canonicalize()?.display())?;
    }
    fs::write(&list, s)?;
    let joined = work.join("joined.mp4");
    util::run(ffmpeg().args(["-f", "concat", "-safe", "0", "-i"]).arg(&list).args(["-c", "copy"]).arg(&joined))?;

    let loud = "loudnorm=I=-14:TP=-1.5:LRA=11";
    let m = &p.forge.music;
    let music = if m.file.is_empty() { None } else { Some(p.root.join(&m.file)) };
    let mut cmd = ffmpeg();
    cmd.arg("-i").arg(&joined);
    match music {
        Some(f) if f.exists() => {
            cmd.args(["-stream_loop", "-1", "-i"]).arg(&f).args([
                "-filter_complex",
                &format!(
                    "[1:a]volume={v}[m];[m][0:a]sidechaincompress=threshold=0.03:ratio=8:attack=20:release=500[duck];\
                     [0:a][duck]amix=inputs=2:duration=first:normalize=0,{loud}[a]",
                    v = m.volume
                ),
                "-map", "0:v", "-map", "[a]",
            ]);
        }
        Some(f) => anyhow::bail!("music file {} not found", f.display()),
        None => {
            cmd.args(["-af", loud, "-map", "0:v", "-map", "0:a"]);
        }
    }
    util::run(cmd.args(["-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-movflags", "+faststart"]).arg(out))?;
    let _ = fs::remove_file(joined);
    Ok(())
}

/// Caption chunks timed across each shot's speech, proportional to text length.
/// Exact for TTS-length estimates; for recorded voice, re-time with whisper.cpp if you want word-level sync.
pub fn captions(p: &Project, v: &Video, timing: &[ShotTiming], out: &Path) -> Result<()> {
    let mut srt = String::new();
    let mut n = 1;
    for (shot, t) in v.shots.iter().zip(timing) {
        let chunks = chunk_text(&shot.narration, 8, 42);
        let total: usize = chunks.iter().map(|c| c.len()).sum::<usize>().max(1);
        let mut at = t.start + p.forge.voice.lead_in;
        for c in chunks {
            let d = t.voice_seconds * c.len() as f64 / total as f64;
            writeln!(srt, "{n}\n{} --> {}\n{c}\n", srt_time(at), srt_time(at + d))?;
            at += d;
            n += 1;
        }
    }
    fs::write(out, srt)?;
    Ok(())
}

fn chunk_text(text: &str, max_words: usize, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for w in text.split_whitespace() {
        let len: usize = cur.iter().map(|x| x.len() + 1).sum::<usize>() + w.len();
        if !cur.is_empty() && (cur.len() >= max_words || len > max_chars) {
            out.push(cur.join(" "));
            cur.clear();
        }
        cur.push(w);
        if w.ends_with(['.', '?', '!', ':', ';']) {
            out.push(cur.join(" "));
            cur.clear();
        }
    }
    if !cur.is_empty() {
        out.push(cur.join(" "));
    }
    out
}

fn stamp(t: f64) -> String {
    let s = t.floor() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Description with chapters appended. YouTube needs >= 3 chapters, the first at 0:00, each >= 10 s.
pub fn description(v: &Video, timing: &[ShotTiming]) -> (String, Vec<String>) {
    let mut chapters: Vec<(f64, String)> = Vec::new();
    for (shot, t) in v.shots.iter().zip(timing) {
        if let Some(c) = &shot.chapter {
            chapters.push((t.start, c.clone()));
        }
    }
    let total: f64 = timing.iter().map(|t| t.seconds).sum();
    let mut warnings = Vec::new();
    let mut ok = chapters.len() >= 3 && chapters[0].0 == 0.0;
    for (i, (s, name)) in chapters.iter().enumerate() {
        let next = chapters.get(i + 1).map(|c| c.0).unwrap_or(total);
        if next - s < 10.0 {
            warnings.push(format!("chapter '{name}' is {:.1}s long; YouTube needs 10s", next - s));
            ok = false;
        }
    }
    if !chapters.is_empty() && chapters[0].0 != 0.0 {
        warnings.push("first chapter must start at 0:00 (put a chapter on the first shot)".into());
    }
    let mut d = v.video.description.trim_end().to_string();
    if ok {
        d.push_str("\n\nChapters\n");
        for (s, name) in &chapters {
            let _ = writeln!(d, "{} {}", stamp(*s), name);
        }
    } else if chapters.len() >= 1 {
        warnings.push("chapters skipped: need at least 3, first at 0:00, each 10s+".into());
    }
    (d, warnings)
}

pub fn thumbnail(p: &Project, still: &Path, text: &str, out: &Path) -> Result<()> {
    let txt = out.with_extension("txt");
    let text = text.to_uppercase();
    // Bold caps run ~0.73 em per character; size the line to fit 1280px with margins.
    let size = (1130.0 / (text.chars().count().max(1) as f64 * 0.73)).min(110.0).round() as u32;
    fs::write(&txt, &text)?;
    let filter = format!(
        "scale=1280:720:flags=lanczos,drawtext=fontfile={font}:textfile={tf}:fontsize={size}:fontcolor=white:\
         x=56:y=h-th-64:box=1:boxcolor=black@0.55:boxborderw=22:line_spacing=10",
        font = p.forge.thumbnail.font,
        tf = txt.display()
    );
    util::run(ffmpeg().arg("-i").arg(still).args(["-vf", &filter, "-frames:v", "1", "-q:v", "2"]).arg(out))?;
    let _ = fs::remove_file(txt);
    Ok(())
}

pub fn metadata(v: &Video, description: &str, out: &Path) -> Result<()> {
    let m = &v.video;
    let doc = json!({
        "title": m.title,
        "description": description,
        "tags": m.tags,
        "categoryId": m.category_id,
        "madeForKids": m.made_for_kids,
        "containsSyntheticMedia": m.contains_synthetic_media,
        "status": m.status,
    });
    fs::write(out, serde_json::to_string_pretty(&doc)?)?;
    Ok(())
}
