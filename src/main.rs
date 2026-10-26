//! forge: the Everyday Machines production pipeline.
//!
//!   forge plan   <video>            shot list, timing and voice sources
//!   forge build  <video> [-p prof]  voice -> render -> assemble -> captions -> thumbnail
//!   forge render <video> --only <shot>   iterate on one shot
//!   forge new    <video> --scene <scene>  start a new video from the template
//!   forge auth                      one-time YouTube OAuth
//!   forge upload <video>            private draft upload (needs status = "ready")

mod assemble;
mod config;
mod render;
mod util;
mod voice;
mod youtube;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use config::{Project, Video};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "forge", about = "Everyday Machines: spec -> voice -> Blender -> ffmpeg -> YouTube draft")]
struct Cli {
    /// Repository root (where forge.toml lives).
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show shots, durations and where each shot's voice comes from.
    Plan { video: String, #[arg(short, long, default_value = "animatic")] profile: String },
    /// Generate/refresh narration audio only.
    Voice { video: String },
    /// Render frames (cached). Use --only to iterate on one shot.
    Render {
        video: String,
        #[arg(short, long, default_value = "animatic")] profile: String,
        #[arg(long)] only: Option<String>,
        #[arg(long)] force: bool,
    },
    /// Full pipeline: voice, render, clips, master, captions, thumbnail, metadata.
    Build {
        video: String,
        #[arg(short, long, default_value = "animatic")] profile: String,
        #[arg(long)] force: bool,
    },
    /// Scaffold videos/<video>/ for a scene.
    New { video: String, #[arg(long)] scene: String, #[arg(long)] title: Option<String> },
    /// One-time OAuth to get a refresh token for the channel.
    Auth,
    /// Upload the final build as a PRIVATE draft (requires status = "ready": script, facts and voice checked).
    Upload { video: String, #[arg(short, long, default_value = "final")] profile: String },
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let root = cli.root.canonicalize().with_context(|| format!("root {}", cli.root.display()))?;
    match cli.cmd {
        Cmd::New { video, scene, title } => new_video(&root, &video, &scene, title),
        Cmd::Auth => youtube::auth(&root),
        cmd => {
            let p = Project::load(&root)?;
            match cmd {
                Cmd::Plan { video, profile } => plan(&p, &video, &profile),
                Cmd::Voice { video } => {
                    let v = p.load_video(&video)?;
                    let t = voice::prepare(&p, &video, &v, 30)?;
                    for s in t {
                        println!("{:<14} {:>6.1}s  {}", s.id, s.voice_seconds, s.source);
                    }
                    Ok(())
                }
                Cmd::Render { video, profile, only, force } => {
                    let v = p.load_video(&video)?;
                    let prof = p.profile(&profile)?;
                    let timing = voice::prepare(&p, &video, &v, prof.fps)?;
                    let frames = p.build_dir(&video, &profile).join("frames");
                    let jobs = render::plan(&p, &v, &prof, &timing, &frames, only.as_deref())?;
                    if jobs.is_empty() {
                        bail!("no shot named {:?}", only);
                    }
                    render::render_all(&p, jobs, force)
                }
                Cmd::Build { video, profile, force } => build(&p, &video, &profile, force),
                Cmd::Upload { video, profile } => upload(&p, &video, &profile),
                Cmd::New { .. } | Cmd::Auth => unreachable!(),
            }
        }
    }
}

fn plan(p: &Project, id: &str, profile: &str) -> Result<()> {
    let v = p.load_video(id)?;
    let prof = p.profile(profile)?;
    let timing = voice::prepare(p, id, &v, prof.fps)?;
    let frames = p.build_dir(id, profile).join("frames");
    let jobs = render::plan(p, &v, &prof, &timing, &frames, None)?;
    println!("{}  [{}]  scene={}  {}x{}@{}", v.video.title, v.video.status, v.video.scene, prof.width, prof.height, prof.fps);
    println!("{:<14} {:>7} {:>7} {:>6}  {:<9} {}", "shot", "start", "length", "frames", "voice", "render");
    for (t, j) in timing.iter().zip(&jobs) {
        println!(
            "{:<14} {:>7} {:>6.1}s {:>6}  {:<9} {}",
            t.id, util::srt_time(t.start)[3..8].to_string(), t.seconds, t.frames, t.source,
            if render::is_current(j) { "cached" } else { "needed" }
        );
    }
    let total: f64 = timing.iter().map(|t| t.seconds).sum();
    println!("total {:.0}s ({:.1} min)", total, total / 60.0);
    Ok(())
}

fn build(p: &Project, id: &str, profile: &str, force: bool) -> Result<()> {
    let t0 = Instant::now();
    let v = p.load_video(id)?;
    let prof = p.profile(profile)?;
    let dir = p.build_dir(id, profile);
    fs::create_dir_all(&dir)?;

    println!("[voice] preparing narration");
    let timing = voice::prepare(p, id, &v, prof.fps)?;
    voice::save_timing(&dir.join("timing.json"), &timing)?;

    let frames = dir.join("frames");
    let jobs = render::plan(p, &v, &prof, &timing, &frames, None)?;
    render::render_all(p, jobs, force)?;

    println!("[assemble] encoding clips and mastering audio");
    let clips = assemble::clips(p, id, &prof, &timing, &frames, &dir.join("clips"))?;
    let out = dir.join(format!("{id}.mp4"));
    assemble::master(p, &clips, &dir, &out)?;
    assemble::captions(p, &v, &timing, &dir.join(format!("{id}.srt")))?;

    let (description, warnings) = assemble::description(&v, &timing);
    assemble::metadata(&v, &description, &dir.join("metadata.json"))?;

    if let Some(th) = &v.thumbnail {
        println!("[thumbnail] rendering still from '{}'", th.shot);
        let t = timing.iter().find(|t| t.id == th.shot).with_context(|| format!("thumbnail shot '{}' not in shots", th.shot))?;
        let frame = 1 + ((t.frames - 1) as f64 * th.at.clamp(0.0, 1.0)).round() as u32;
        let still = render::still(p, &v, &prof, t, frame, &dir.join("thumb"))?;
        assemble::thumbnail(p, &still, &th.text, &dir.join("thumbnail.jpg"))?;
    }

    let silent: Vec<&str> = timing.iter().filter(|t| t.source == "silent").map(|t| t.id.as_str()).collect();
    println!("\nbuilt {} ({:.0}s) in {:.0}s", out.display(), util::duration(&out)?, t0.elapsed().as_secs_f64());
    if !silent.is_empty() {
        println!("  note: no voice yet for {} -> record videos/{id}/vo/<shot>.wav (or configure piper)", silent.join(", "));
    }
    for w in warnings {
        println!("  note: {w}");
    }
    if v.video.status == "draft" {
        println!("  status = \"draft\": once the script, sources.md and voice are checked, set status = \"ready\" to allow a private upload");
    }
    Ok(())
}

fn upload(p: &Project, id: &str, profile: &str) -> Result<()> {
    let v = p.load_video(id)?;
    if !matches!(v.video.status.as_str(), "ready" | "published") {
        bail!("{id} is '{}'. Check the script against sources.md and record the voice, then set status = \"ready\" in video.toml.", v.video.status);
    }
    let dir = p.build_dir(id, profile);
    let video = dir.join(format!("{id}.mp4"));
    if !video.exists() {
        bail!("{} not found: run `forge build {id} -p {profile}` first", video.display());
    }
    let meta: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("metadata.json"))?)?;
    let srt = dir.join(format!("{id}.srt"));
    let thumb = dir.join("thumbnail.jpg");
    let vid = youtube::upload(
        &p.root,
        youtube::UploadInput {
            video: &video,
            srt: srt.exists().then_some(srt.as_path()),
            thumbnail: thumb.exists().then_some(thumb.as_path()),
            metadata: &meta,
        },
    )?;
    fs::write(dir.join("upload.json"), serde_json::json!({"video_id": vid}).to_string())?;
    println!("draft ready: https://studio.youtube.com/video/{vid}/edit  (private; publish it from Studio)");
    Ok(())
}

fn new_video(root: &Path, id: &str, scene: &str, title: Option<String>) -> Result<()> {
    let dir = root.join("videos").join(id);
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    if !root.join("blender/scenes").join(format!("{scene}.py")).exists() {
        bail!("blender/scenes/{scene}.py doesn't exist yet. Write the scene first (copy pin_tumbler.py as a start).");
    }
    fs::create_dir_all(dir.join("vo"))?;
    let v = Video {
        video: config::Meta {
            title: title.unwrap_or_else(|| "TITLE".into()),
            description: "One or two sentences on what this video explains.\n\nSources: see sources.md".into(),
            tags: vec!["how it works".into(), "engineering".into(), "mechanism".into()],
            category_id: "28".into(),
            scene: scene.into(),
            scene_params: None,
            status: "draft".into(),
            made_for_kids: false,
            contains_synthetic_media: false,
        },
        shots: vec![config::Shot {
            id: "intro".into(),
            narration: "Write the narration for this shot.".into(),
            chapter: Some("Intro".into()),
            min_seconds: 3.0,
            pad_seconds: 0.6,
        }],
        thumbnail: None,
    };
    fs::write(dir.join("video.toml"), toml::to_string_pretty(&v)?)?;
    fs::write(dir.join("sources.md"), "# Sources\n\nList every source you checked facts against.\n")?;
    println!("created {}", dir.display());
    Ok(())
}

