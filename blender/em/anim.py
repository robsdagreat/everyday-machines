"""Animation helpers: keyframes, easing, camera rigs, labels.

Shots are written as plain Python timelines, e.g.

    t = Timeline(frames)            # frames = shot length decided by the narration
    a, b = t.span(0.1, 0.4)         # 10%..40% of the shot
    anim.loc(obj, a, (0, 0, 0)); anim.loc(obj, b, (0, -3, 0))

so a shot automatically stretches or squeezes to fit its voiceover.
"""
import math

import bpy
from mathutils import Vector

from . import style


class Timeline:
    """Maps fractions of a shot to frame numbers (frames start at 1)."""

    def __init__(self, frames):
        self.frames = max(1, int(frames))

    def at(self, fraction):
        return 1 + round(min(max(fraction, 0.0), 1.0) * (self.frames - 1))

    def span(self, f0, f1):
        return self.at(f0), self.at(f1)


def smoothstep(x):
    x = min(max(x, 0.0), 1.0)
    return x * x * (3 - 2 * x)


def _set_interp(obj, path, frame, interp):
    ad = obj.animation_data
    if not ad or not ad.action:
        return
    for fc in ad.action.fcurves:
        if fc.data_path == path:
            for kp in fc.keyframe_points:
                if abs(kp.co.x - frame) < 0.5:
                    kp.interpolation = interp


def key(obj, path, frame, value=None, interp="BEZIER"):
    """Set obj.<path> = value (if given) and keyframe it at `frame`."""
    if value is not None:
        setattr(obj, path, value)
    obj.keyframe_insert(data_path=path, frame=frame)
    _set_interp(obj, path, frame, interp)


def loc(obj, frame, xyz=None, interp="BEZIER"):
    key(obj, "location", frame, xyz, interp)


def rot(obj, frame, xyz=None, interp="BEZIER"):
    key(obj, "rotation_euler", frame, xyz, interp)


def scale(obj, frame, s=None, interp="BEZIER"):
    if isinstance(s, (int, float)):
        s = (s, s, s)
    key(obj, "scale", frame, s, interp)


def pop_in(obj, frame, dur=8, size=1.0):
    """Scale from nothing with a small overshoot. Objects start hidden."""
    scale(obj, 1, 0.0)
    scale(obj, frame, 0.0)
    scale(obj, frame + int(dur * 0.7), size * 1.08)
    scale(obj, frame + dur, size)


def pop_out(obj, frame, dur=6):
    scale(obj, frame)
    scale(obj, frame + dur, 0.0)


def wiggle_rotation(obj, axis, frame, dur, degrees=4.0, times=2):
    """A blocked attempt to turn: rotate a little and spring back."""
    idx = {"X": 0, "Y": 1, "Z": 2}[axis]
    base = list(obj.rotation_euler)
    step = dur / (times * 2)
    rot(obj, frame, base)
    for i in range(times):
        r = list(base)
        r[idx] += math.radians(degrees)
        rot(obj, frame + int(step * (2 * i + 1)), r)
        rot(obj, frame + int(step * (2 * i + 2)), base)


# ---------------------------------------------------------------- camera

def camera_rig(target=(0, 0, 0), distance=10.0, elevation_deg=12.0, azimuth_deg=-90.0, lens=50):
    """Camera on a boom around `target`. Animate the pivot's Z rotation to orbit,
    the boom's location to dolly, or the pivot's location to pan."""
    pivot = bpy.data.objects.new("cam_pivot", None)
    pivot.location = target
    bpy.context.collection.objects.link(pivot)

    cam_data = bpy.data.cameras.new("camera")
    cam_data.lens = lens
    cam = bpy.data.objects.new("camera", cam_data)
    bpy.context.collection.objects.link(cam)
    cam.parent = pivot
    set_boom(cam, distance, elevation_deg, azimuth_deg)

    aim = bpy.data.objects.new("cam_aim", None)
    aim.parent = pivot
    bpy.context.collection.objects.link(aim)
    c = cam.constraints.new("TRACK_TO")
    c.target = aim
    c.track_axis = "TRACK_NEGATIVE_Z"
    c.up_axis = "UP_Y"
    bpy.context.scene.camera = cam
    return cam, pivot


def set_boom(cam, distance, elevation_deg, azimuth_deg):
    e, a = math.radians(elevation_deg), math.radians(azimuth_deg)
    cam.location = (distance * math.cos(e) * math.cos(a),
                    distance * math.cos(e) * math.sin(a),
                    distance * math.sin(e))


def boom(cam, frame, distance, elevation_deg, azimuth_deg, interp="BEZIER"):
    set_boom(cam, distance, elevation_deg, azimuth_deg)
    loc(cam, frame, interp=interp)


# ---------------------------------------------------------------- annotation

def label(text, location, size=0.17, color="label", align="CENTER"):
    """Flat text that always faces the camera. Starts hidden; use pop_in()."""
    curve = bpy.data.curves.new(f"label_{text}", "FONT")
    curve.body = text
    curve.size = size
    curve.align_x = align
    curve.align_y = "CENTER"
    obj = bpy.data.objects.new(f"label_{text}", curve)
    obj.location = location
    bpy.context.collection.objects.link(obj)
    obj.data.materials.append(style.glow(color, 2.5))
    cam = bpy.context.scene.camera
    if cam:
        c = obj.constraints.new("TRACK_TO")
        c.target = cam
        c.track_axis = "TRACK_Z"
        c.up_axis = "UP_Y"
    obj.scale = (0, 0, 0)
    return obj


def rod(name, p0, p1, radius, mat):
    """Thin cylinder between two points: leader lines, guide lines, axes."""
    p0, p1 = Vector(p0), Vector(p1)
    d = p1 - p0
    data = bpy.data.meshes.new(name)
    obj = bpy.data.objects.new(name, data)
    bpy.context.collection.objects.link(obj)
    import bmesh
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, segments=16, radius1=radius, radius2=radius, depth=d.length)
    bm.to_mesh(data)
    bm.free()
    obj.location = (p0 + p1) / 2
    obj.rotation_mode = "QUATERNION"
    obj.rotation_quaternion = Vector((0, 0, 1)).rotation_difference(d.normalized())
    obj.data.materials.append(mat)
    return obj
