"""Render one shot of one scene. Called by the Rust orchestrator, one process per shot.

Works both ways:
    blender -b --factory-startup --python blender/render_shot.py -- --scene pin_tumbler --shot reveal ...
    python3.11 blender/render_shot.py --scene pin_tumbler --shot reveal ...   (with `pip install bpy`)
"""
import argparse
import importlib
import json
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import bpy  # noqa: E402

from em import anim, style  # noqa: E402


def parse():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
    p = argparse.ArgumentParser()
    p.add_argument("--scene", required=True, help="module in blender/scenes/")
    p.add_argument("--shot", required=True)
    p.add_argument("--frames", type=int, required=True)
    p.add_argument("--fps", type=int, default=30)
    p.add_argument("--width", type=int, default=1920)
    p.add_argument("--height", type=int, default=1080)
    p.add_argument("--quality", choices=["preview", "draft", "lean", "final"], default="preview")
    p.add_argument("--engine", choices=["cycles", "eevee", "workbench"], default="cycles")
    p.add_argument("--gpu", action="store_true")
    p.add_argument("--out", required=True, help="output directory for frames")
    p.add_argument("--still", type=int, help="render only this frame to <out>/still.png")
    p.add_argument("--params", default="{}", help="JSON passed to scene.build()")
    p.add_argument("--save-blend", action="store_true", help="also write shot.blend for inspection")
    p.add_argument("--list-shots", action="store_true")
    return p.parse_args(argv)


def main():
    a = parse()
    scene_mod = importlib.import_module(f"scenes.{a.scene}")
    if a.list_shots:
        print(json.dumps(sorted(scene_mod.SHOTS)))
        return
    if a.shot not in scene_mod.SHOTS:
        sys.exit(f"unknown shot '{a.shot}'. available: {', '.join(sorted(scene_mod.SHOTS))}")

    bpy.ops.wm.read_factory_settings(use_empty=True)
    style.setup_world()
    style.setup_render(a.width, a.height, a.fps, a.frames, a.quality, a.engine, a.gpu)

    ctx = scene_mod.build(json.loads(a.params))
    scene_mod.SHOTS[a.shot](ctx, anim.Timeline(a.frames))

    os.makedirs(a.out, exist_ok=True)
    scene = bpy.context.scene
    if a.save_blend:
        bpy.ops.wm.save_as_mainfile(filepath=os.path.join(a.out, "shot.blend"))

    t0 = time.time()
    if a.still is not None:
        scene.frame_set(min(max(a.still, 1), a.frames))
        scene.render.filepath = os.path.join(a.out, "still.png")
        bpy.ops.render.render(write_still=True)
    else:
        scene.render.filepath = os.path.join(a.out, "f_")
        bpy.ops.render.render(animation=True)
    print(f"FORGE_DONE shot={a.shot} frames={a.frames} seconds={time.time() - t0:.1f}", flush=True)


if __name__ == "__main__":
    main()
