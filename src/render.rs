//! Blender rendering: one process per shot, `jobs` at a time, cached by content hash.
//!
//! A shot is re-rendered only when its inputs change: the scene file, the toolkit,
//! the shot length (i.e. the narration), the profile or the scene params.

use crate::config::{Profile, Project, Video};
use crate::util::{self, FNV_SEED};
use crate::voice::ShotTiming;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct Job {
    pub shot: String,
    pub args: Vec<String>,
    pub out: PathBuf,
    pub frames: u32,
    pub key: String,
    /// Frames this job renders (1-based, inclusive). A whole shot unless the build is sharded.
    pub range: (u32, u32),
}

fn toolkit_hash(p: &Project, scene: &str) -> Result<u64> {
    let b = p.root.join("blender");
    let mut files = vec![b.join("render_shot.py"), b.join("scenes").join(format!("{scene}.py"))];
    let mut em: Vec<PathBuf> = fs::read_dir(b.join("em"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|f| f.extension().map(|x| x == "py").unwrap_or(false))
        .collect();
    em.sort();
    files.extend(em);
    let mut h = FNV_SEED;
    for f in files {
        let bytes = fs::read(&f).with_context(|| format!("reading {}", f.display()))?;
        h = util::fnv1a(&bytes, h);
    }
    Ok(h)
}

pub fn scene_params_json(v: &Video) -> Result<String> {
    Ok(match &v.video.scene_params {
        Some(t) => serde_json::to_string(t)?,
        None => "{}".into(),
    })
}

fn base_args(p: &Project, v: &Video, prof: &Profile) -> Result<Vec<String>> {
    let r = &p.forge.render;
    let mut a = vec![
        "--scene".into(), v.video.scene.clone(),
        "--fps".into(), prof.fps.to_string(),
        "--width".into(), prof.width.to_string(),
        "--height".into(), prof.height.to_string(),
        "--quality".into(), prof.quality.clone(),
        "--engine".into(), prof.engine.clone().unwrap_or_else(|| r.engine.clone()),
        "--params".into(), scene_params_json(v)?,
    ];
    if r.gpu {
        a.push("--gpu".into());
    }
    Ok(a)
}

/// Which slice of the whole video's frames one machine renders: `index` of `count`, 0-based.
#[derive(Debug, Clone, Copy)]
pub struct Shard {
    pub index: u32,
    pub count: u32,
}

impl std::str::FromStr for Shard {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (i, n) = s.split_once('/').ok_or("expected INDEX/COUNT, e.g. 0/8")?;
        let (index, count): (u32, u32) = (i.parse().map_err(|_| "bad shard index")?, n.parse().map_err(|_| "bad shard count")?);
        if count == 0 || index >= count {
            return Err(format!("shard index must be below the count (got {index}/{count})"));
        }
        Ok(Shard { index, count })
    }
}

/// `count` is chosen so no shard exceeds `frames_per_shard` frames.
pub fn shard_count(timing: &[ShotTiming], frames_per_shard: u32) -> u32 {
    let total: u32 = timing.iter().map(|t| t.frames).sum();
    total.div_ceil(frames_per_shard.max(1)).max(1)
}

pub fn plan(p: &Project, v: &Video, prof: &Profile, timing: &[ShotTiming], frames_root: &Path, only: Option<&str>, shard: Option<Shard>) -> Result<Vec<Job>> {
    let tk = toolkit_hash(p, &v.video.scene)?;
    let base = base_args(p, v, prof)?;
    let total: u32 = timing.iter().map(|t| t.frames).sum();
    // The shard owns global frames [lo, hi), counted across all shots in order.
    let (lo, hi) = match shard {
        Some(s) => (
            (total as u64 * s.index as u64 / s.count as u64) as u32,
            (total as u64 * (s.index as u64 + 1) / s.count as u64) as u32,
        ),
        None => (0, total),
    };
    let mut jobs = Vec::new();
    let mut offset = 0;
    for t in timing {
        let (first, last) = (offset + 1, offset + t.frames);
        offset += t.frames;
        if only.map(|o| o != t.id).unwrap_or(false) {
            continue;
        }
        let before = (offset - t.frames) as i64;
        let (start, end) = (first.max(lo + 1) as i64 - before, last.min(hi) as i64 - before);
        if start > end {
            continue;
        }
        let (start, end) = (start as u32, end as u32);
        let out = frames_root.join(&t.id);
        let mut args = base.clone();
        args.extend(["--shot".into(), t.id.clone(), "--frames".into(), t.frames.to_string(), "--out".into(), out.display().to_string()]);
        // The key ignores the range, so every shard of a shot stamps the same key and the pieces merge as one cached shot.
        let key = format!("{:016x}", util::fnv1a(args.join("\u{1f}").as_bytes(), tk));
        if (start, end) != (1, t.frames) {
            args.extend(["--start".into(), start.to_string(), "--end".into(), end.to_string()]);
        }
        jobs.push(Job { shot: t.id.clone(), args, out, frames: t.frames, key, range: (start, end) });
    }
    Ok(jobs)
}

pub fn is_current(job: &Job) -> bool {
    let stamp_ok = fs::read_to_string(job.out.join(".stamp")).ok().as_deref() == Some(job.key.as_str());
    stamp_ok && (job.range.0..=job.range.1).all(|n| job.out.join(format!("f_{n:04}.png")).exists())
}

fn blender_cmd(p: &Project, args: &[String]) -> Result<Command> {
    let r = &p.forge.render;
    let Some((prog, pre)) = r.blender.split_first() else { bail!("render.blender is empty in forge.toml") };
    let mut c = Command::new(prog);
    c.args(pre).arg(p.root.join("blender").join("render_shot.py")).arg("--").args(args);
    Ok(c)
}

fn run_job(p: &Project, job: &Job) -> Result<()> {
    if job.range == (1, job.frames) && job.out.exists() {
        for e in fs::read_dir(&job.out)? {
            let f = e?.path();
            if f.file_name().map(|n| n.to_string_lossy().starts_with("f_")).unwrap_or(false) {
                fs::remove_file(f)?;
            }
        }
    }
    fs::create_dir_all(&job.out)?;
    let log = fs::File::create(job.out.join("render.log"))?;
    let status = blender_cmd(p, &job.args)?
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .status()
        .context("starting Blender (check render.blender in forge.toml)")?;
    if !status.success() || !job.out.join(format!("f_{:04}.png", job.range.1)).exists() {
        let text = fs::read_to_string(job.out.join("render.log")).unwrap_or_default();
        let tail: Vec<&str> = text.lines().rev().filter(|l| !l.starts_with("Fra:")).take(12).collect();
        bail!("shot '{}' failed:\n{}", job.shot, tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    fs::write(job.out.join(".stamp"), &job.key)?;
    Ok(())
}

/// Render every job that isn't cached, `jobs` processes at a time.
pub fn render_all(p: &Project, jobs: Vec<Job>, force: bool) -> Result<()> {
    let todo: Vec<Job> = jobs.into_iter().filter(|j| force || !is_current(j)).collect();
    if todo.is_empty() {
        println!("[render] all shots up to date");
        return Ok(());
    }
    let total = todo.len();
    let queue = Arc::new(Mutex::new(todo));
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    let workers = p.forge.render.jobs.max(1).min(total);
    std::thread::scope(|s| {
        for _ in 0..workers {
            let (queue, errors) = (Arc::clone(&queue), Arc::clone(&errors));
            s.spawn(move || loop {
                let Some(job) = queue.lock().unwrap().pop() else { break };
                let t0 = Instant::now();
                println!("[render] {} (frames {}-{}) ...", job.shot, job.range.0, job.range.1);
                match run_job(p, &job) {
                    Ok(()) => println!("[render] {} done in {:.0}s", job.shot, t0.elapsed().as_secs_f64()),
                    Err(e) => errors.lock().unwrap().push(format!("{e:#}")),
                }
            });
        }
    });
    let errors = errors.lock().unwrap();
    if !errors.is_empty() {
        bail!("{} of {} shots failed:\n{}", errors.len(), total, errors.join("\n\n"));
    }
    Ok(())
}

/// Render one frame of a shot to `out_dir/still.png` (used for thumbnails).
pub fn still(p: &Project, v: &Video, prof: &Profile, t: &ShotTiming, frame: u32, out_dir: &Path) -> Result<PathBuf> {
    let mut args = base_args(p, v, prof)?;
    args.extend([
        "--shot".into(), t.id.clone(), "--frames".into(), t.frames.to_string(),
        "--still".into(), frame.to_string(), "--out".into(), out_dir.display().to_string(),
    ]);
    fs::create_dir_all(out_dir)?;
    util::run(&mut blender_cmd(p, &args)?)?;
    Ok(out_dir.join("still.png"))
}
