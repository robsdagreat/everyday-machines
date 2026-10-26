//! forge.toml (pipeline settings) and videos/<id>/video.toml (one video's spec).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Forge {
    pub render: RenderCfg,
    pub profiles: BTreeMap<String, Profile>,
    #[serde(default)]
    pub voice: VoiceCfg,
    #[serde(default)]
    pub music: MusicCfg,
    #[serde(default)]
    pub thumbnail: ThumbCfg,
}

#[derive(Debug, Deserialize)]
pub struct RenderCfg {
    /// Command prefix that runs a Python file inside Blender, e.g.
    /// ["blender", "-b", "--factory-startup", "--python"] or ["python3.11"] (bpy module).
    pub blender: Vec<String>,
    #[serde(default = "one")]
    pub jobs: usize,
    #[serde(default = "cycles")]
    pub engine: String,
    #[serde(default)]
    pub gpu: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Profile {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub quality: String,
    /// Overrides render.engine for this profile (e.g. "workbench" for animatics).
    #[serde(default)]
    pub engine: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VoiceCfg {
    /// Path to the piper binary; empty = don't use TTS.
    #[serde(default)]
    pub piper: String,
    #[serde(default)]
    pub piper_model: String,
    /// Used to time silent placeholders when no recording or TTS exists.
    #[serde(default = "wpm")]
    pub words_per_minute: f64,
    /// Seconds of silence before the narration starts in each shot.
    #[serde(default = "lead_in")]
    pub lead_in: f64,
}

impl Default for VoiceCfg {
    fn default() -> Self {
        Self { piper: String::new(), piper_model: String::new(), words_per_minute: wpm(), lead_in: lead_in() }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct MusicCfg {
    #[serde(default)]
    pub file: String,
    #[serde(default = "music_vol")]
    pub volume: f64,
}

#[derive(Debug, Deserialize)]
pub struct ThumbCfg {
    #[serde(default = "font")]
    pub font: String,
}

impl Default for ThumbCfg {
    fn default() -> Self {
        Self { font: font() }
    }
}

fn one() -> usize { 1 }
fn cycles() -> String { "cycles".into() }
fn wpm() -> f64 { 150.0 }
fn lead_in() -> f64 { 0.25 }
fn music_vol() -> f64 { 0.12 }
fn font() -> String { "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".into() }

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Video {
    pub video: Meta,
    pub shots: Vec<Shot>,
    pub thumbnail: Option<Thumb>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Meta {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// YouTube category; 28 = Science & Technology, 27 = Education.
    #[serde(default = "cat")]
    pub category_id: String,
    /// Blender scene module in blender/scenes/.
    pub scene: String,
    /// JSON object passed to the scene's build(); lets one scene make many variants.
    #[serde(default)]
    pub scene_params: Option<toml::Table>,
    /// "draft" until a human has watched the final cut. Upload refuses drafts.
    #[serde(default = "draft")]
    pub status: String,
    #[serde(default)]
    pub made_for_kids: bool,
    /// YouTube's altered/synthetic content flag: realistic synthetic people, places or events.
    /// Stylised motion graphics usually don't need it; a cloned real voice might.
    #[serde(default)]
    pub contains_synthetic_media: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Shot {
    pub id: String,
    pub narration: String,
    /// Starts a YouTube chapter here (added to the description as a timestamp).
    #[serde(default)]
    pub chapter: Option<String>,
    #[serde(default = "min_s")]
    pub min_seconds: f64,
    #[serde(default = "pad_s")]
    pub pad_seconds: f64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Thumb {
    pub shot: String,
    /// Position in the shot, 0.0..1.0.
    #[serde(default = "end")]
    pub at: f64,
    pub text: String,
}

fn cat() -> String { "28".into() }
fn draft() -> String { "draft".into() }
fn min_s() -> f64 { 3.0 }
fn pad_s() -> f64 { 0.6 }
fn end() -> f64 { 1.0 }

pub struct Project {
    pub root: PathBuf,
    pub forge: Forge,
}

impl Project {
    pub fn load(root: &Path) -> Result<Self> {
        let p = root.join("forge.toml");
        let text = fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let mut forge: Forge = toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))?;
        // Per-machine override without editing forge.toml, e.g. FORGE_BLENDER="python3.11"
        if let Ok(b) = std::env::var("FORGE_BLENDER") {
            forge.render.blender = b.split_whitespace().map(String::from).collect();
        }
        Ok(Self { root: root.to_path_buf(), forge })
    }

    pub fn profile(&self, name: &str) -> Result<Profile> {
        match self.forge.profiles.get(name) {
            Some(p) => Ok(p.clone()),
            None => bail!("no profile '{name}' in forge.toml (have: {:?})", self.forge.profiles.keys().collect::<Vec<_>>()),
        }
    }

    pub fn video_dir(&self, id: &str) -> PathBuf {
        self.root.join("videos").join(id)
    }

    pub fn build_dir(&self, id: &str, profile: &str) -> PathBuf {
        self.root.join("build").join(id).join(profile)
    }

    pub fn load_video(&self, id: &str) -> Result<Video> {
        let p = self.video_dir(id).join("video.toml");
        let text = fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let v: Video = toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))?;
        let mut seen = std::collections::HashSet::new();
        for s in &v.shots {
            if !seen.insert(&s.id) {
                bail!("duplicate shot id '{}'", s.id);
            }
        }
        Ok(v)
    }
}
