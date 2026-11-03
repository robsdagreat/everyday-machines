# Everyday Machines

Production pipeline for a YouTube channel that explains everyday mechanisms with 3D motion graphics.

```
videos/<id>/video.toml ──► voice ──► Blender (headless, cached) ──► ffmpeg ──► <id>.mp4 + .srt + thumbnail + metadata ──► private YouTube draft
     script + shots        per shot     one process per shot          clips, music, loudness            you review, then publish
```

**The rule this repo is built around:** machines do the production, a person owns the story and the final check.
The script, the voice and the publish click are human. Everything between is automated. That split is what keeps the
channel clear of YouTube's "inauthentic content" rules, and it's also what makes the videos good.

## Setup

| Tool | Version | Notes |
|---|---|---|
| Blender | 4.5 LTS | `blender` on PATH. Or `pip install bpy==4.5.*` on Python 3.11 and set `FORGE_BLENDER=python3.11` |
| ffmpeg / ffprobe | 6+ | |
| Rust | stable | `cargo build --release` → `target/release/forge` |
| piper (optional) | | Local TTS for scratch narration. Your own recorded voice is the real thing. |

```bash
cargo build --release
alias forge=./target/release/forge
forge plan 001-pin-tumbler-lock          # shots, timing, voice source, cache state
forge build 001-pin-tumbler-lock         # animatic (flat shading, fast)
forge build 001-pin-tumbler-lock -p final
```

With pnpm (builds `forge` on first use and rebuilds when `src/` changes):

```bash
pnpm install
pnpm plan 001-pin-tumbler-lock           # same as forge plan
pnpm animatic 001-pin-tumbler-lock       # forge build -p animatic
pnpm final 001-pin-tumbler-lock          # forge build -p final
pnpm forge render 001-pin-tumbler-lock --only <shot>   # any other subcommand
```

Output lands in `build/<id>/<profile>/`: the video, captions (`.srt`), `thumbnail.jpg`, `metadata.json` (with chapters), and per-shot frames and logs.

## Making a video

1. **Pick the mechanism, research it.** Fill in `videos/<id>/sources.md` first: every claim gets a source.
2. **Write the scene** in `blender/scenes/<name>.py` (see below). This is where most of the work goes, and every scene adds reusable parts to the toolkit.
3. **Write the script** in `video.toml`, one `[[shots]]` entry per shot. Shot length follows the narration, so you never time anything by hand.
4. **Animatic:** `forge build <id>` renders flat-shaded at about 1 s/frame. Watch it, fix timing and framing.
   Iterate on one shot with `forge render <id> --only <shot>`. Unchanged shots are cached.
5. **Record narration:** `videos/<id>/vo/<shot>.wav`, one file per shot. Recordings override TTS and placeholders; leading/trailing silence is trimmed automatically.
6. **Final:** `forge build <id> -p final`. Watch the whole thing. Fix what's wrong.
7. **Ready:** set `status = "ready"` in `video.toml` once script, facts and voice are checked. `forge upload <id>` uploads a **private** draft with captions and thumbnail.
8. **Review and publish** in YouTube Studio: watch the private draft end to end, then publish. Set `status = "published"` afterwards.

## Profiles (`forge.toml`)

| Profile | Engine | Size | Use |
|---|---|---|---|
| `animatic` | Workbench | 640×360 @24 | Timing, motion, framing. Fast on any CPU. |
| `preview` | Cycles, 12 spp | 640×360 @24 | Check lighting and materials. |
| `draft` | Cycles, 48 spp | 1280×720 @30 | Share with your friend for notes. |
| `final` | Cycles, 16 spp + denoise | 1920×1080 @30 | Upload. Nearly identical to 160 spp at about 1/5 the time. |
| `max` | Cycles, 160 spp | 1920×1080 @30 | Only on a GPU machine (`gpu = true`). |

Measured on a 2-core cloud VM: animatic ≈ 0.3 s/frame, `final` ≈ 45 s/frame. Scale roughly by core count;
an NVIDIA GPU with `gpu = true` is one to two orders of magnitude faster.

## Hosted pipeline (GitHub, free)

Everything is driven from a **private GitHub repo**, from a browser or the GitHub mobile app:

```
edit video.toml / upload vo/*.wav on github.com
        │
        ├─► push ─► animatic workflow ─► GitHub-hosted runner (free minutes) ─► preview mp4 as a download (7 days)
        │
        └─► Actions ▸ final ▸ Run ─► YOUR self-hosted runner (free, no minute limit) ─► 1080p render ─► private YouTube draft
                                                                                            └─► you watch + publish in Studio
```

**Why two kinds of machine.** Private repos get 2,000 free GitHub-hosted minutes a month and jobs there stop at 6 hours.
An animatic costs ~15–25 minutes, so that's ~80+ animatics a month. A 1080p render is many hours of CPU, so it runs on a
machine you register as a *self-hosted runner*: those use no GitHub minutes and have no 6-hour limit. Any spare Linux
machine works (an old laptop, a PC running WSL2, a free cloud VM). It only makes outbound connections to GitHub.

**Set up**

1. Create a **private** repo, push this code.
2. Repo ▸ Settings ▸ Secrets and variables ▸ Actions: add `YT_CLIENT_ID`, `YT_CLIENT_SECRET`, `YT_REFRESH_TOKEN`
   (get the refresh token once with `forge auth` on any computer; see below).
3. Repo ▸ Settings ▸ Actions ▸ Runners ▸ *New self-hosted runner*: copy the token, then on the render machine:
   `bash deploy/setup-runner.sh https://github.com/OWNER/REPO <TOKEN>`
4. Push a change to `videos/` → watch the `animatic` run → download the preview from the run page.
5. Actions ▸ `final` ▸ *Run workflow* → pick the video, tick *upload* when `status = "ready"`.

**Free render machine options**

| Machine | Cost | Notes |
|---|---|---|
| A spare PC or laptop (Linux or WSL2) | Free | Best option. A gaming PC with an NVIDIA GPU renders `max` quality quickly. |
| Oracle Cloud Always Free (ARM, 4 cores / 24 GB) | Free | Card needed at signup; ARM capacity is often "out of stock"; the script stops if no bpy build exists for ARM. |
| GitHub-hosted runner | Free minutes | Set `runs-on: ubuntu-24.04` in `final.yml` and use the `draft` profile for short videos only (6 h job limit, 2,000 min/month). |

## Writing a scene

A scene module exposes `build(params) -> ctx` and a `SHOTS` dict of `shot(ctx, timeline)` functions.
Each shot renders in a fresh Blender process, so it sets up its own camera and visibility.

```python
def shot_reveal(ctx, t):                      # t maps fractions of the shot to frames
    cam, pivot = anim.camera_rig((1.3, 0, 0.55), 12, 14, -150, lens=46)
    a, b = t.span(0.05, 0.3)                  # 5%..30% of this shot, whatever its length
    anim.loc(ctx["housing_front"], a, (0, 0, 0))
    anim.loc(ctx["housing_front"], b, (0, -3, 0))
    anim.pop_in(anim.label("plug", (3.6, -0.45, 0.35)), t.at(0.72))
```

Toolkit (`blender/em/`):

- `style` — the channel palette, materials, lighting rig and render settings. Change the look of every video in one place.
- `parts` — procedural geometry: `cylinder`, `box`, `boolean`, `split_section` (cutaways), `spring`, `extrude_profile` (keys, levers, cams), `involute_gear` + `mesh_gear_pair`.
- `anim` — `Timeline`, keyframe helpers, `camera_rig`/`boom`, `label`, `pop_in`/`pop_out`, `wiggle_rotation`, `rod`.

Drive motion from the mechanism's logic, not hand-placed keyframes. In `pin_tumbler.py`, every pin height is
computed from the key's cut profile each frame, so a different key (`scene_params.good_cuts`) re-animates correctly.

## YouTube API setup (one time)

1. Ideally the channel is a **Brand Account** owned by the channel owner, with you added as Manager. As far as I know, that's the cleanest way for an API client signed in as you to upload to his channel. Confirm it works on his channel with a test upload.
2. In Google Cloud Console: create a project → enable **YouTube Data API v3** → OAuth consent screen → create an **OAuth client ID (Desktop app)**.
3. Save `{"client_id": "...", "client_secret": "..."}` to `.secrets/youtube.json` (gitignored).
4. `forge auth` → approve in the browser, choosing the channel. The refresh token is saved.
5. **Apply for the API audit early.** Until the project passes it, API uploads stay locked to private.
6. Quota: the default allowance covers only a handful of uploads per day. That's plenty for this channel.

## Layout

```
forge.toml                 pipeline settings and render profiles
src/                       Rust orchestrator (forge)
blender/render_shot.py     entry point Blender runs for each shot
blender/em/                reusable toolkit: style, parts, anim
blender/scenes/            one module per mechanism
videos/<id>/video.toml     script, shots, metadata, thumbnail
videos/<id>/sources.md     fact check for every claim
videos/<id>/vo/            your narration recordings
build/                     outputs (gitignored)
```
