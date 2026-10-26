"""Channel look: one palette, one set of materials, one lighting rig.

Keep every video on this file so the channel has a consistent visual identity.
Change the palette here, re-render, and the whole back catalogue can follow.
"""
import bpy

PALETTE = {
    "bg": "#0E1A24",          # deep navy background
    "housing": "#7F8C99",     # brushed steel body
    "plug": "#C9A44C",        # brass
    "key_pin": "#E07A3F",     # warm orange: parts the key touches
    "driver_pin": "#3FA7B5",  # teal: parts pushed by springs
    "spring": "#D9DEE3",
    "key": "#AEB6BF",
    "ok": "#5BD46B",
    "bad": "#F0504A",
    "label": "#F4F1E8",
    "highlight": "#FFD23F",   # the one thing to look at
}


def rgba(hex_color, alpha=1.0):
    h = hex_color.lstrip("#")
    srgb = [int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    # Blender colour inputs are linear; convert from sRGB.
    lin = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in srgb]
    return (*lin, alpha)


_MATS = {}


def material(name, color, metallic=0.0, roughness=0.5, emission=0.0):
    """Cached Principled BSDF material. `color` is a palette key or hex string."""
    if name in _MATS:
        return _MATS[name]
    hex_color = PALETTE.get(color, color)
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = rgba(hex_color)
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["Roughness"].default_value = roughness
    if emission > 0:
        bsdf.inputs["Emission Color"].default_value = rgba(hex_color)
        bsdf.inputs["Emission Strength"].default_value = emission
    mat.diffuse_color = rgba(hex_color)  # viewport colour
    _MATS[name] = mat
    return mat


def metal(color, roughness=0.35):
    return material(f"metal_{color}", color, metallic=0.75, roughness=roughness)


def matte(color):
    return material(f"matte_{color}", color, metallic=0.0, roughness=0.6)


def glow(color, strength=3.0):
    return material(f"glow_{color}", color, emission=strength)


def assign(obj, mat):
    obj.data.materials.clear()
    obj.data.materials.append(mat)
    return obj


def setup_world():
    world = bpy.data.worlds.get("World") or bpy.data.worlds.new("World")
    bpy.context.scene.world = world
    world.use_nodes = True
    bg = world.node_tree.nodes["Background"]
    bg.inputs["Color"].default_value = rgba(PALETTE["bg"])
    bg.inputs["Strength"].default_value = 1.0
    world.color = rgba(PALETTE["bg"])[:3]  # used by the workbench (animatic) engine


def setup_lights(target=(0, 0, 0)):
    """Three-point studio rig. Area lights read well on metal at low sample counts."""
    tx, ty, tz = target
    rig = [
        ("key_light", (tx - 4, ty - 6, tz + 6), 900, 4.0),
        ("fill_light", (tx + 6, ty - 4, tz + 2), 300, 6.0),
        ("rim_light", (tx + 1, ty + 6, tz + 5), 600, 3.0),
    ]
    for name, loc, power, size in rig:
        data = bpy.data.lights.new(name, "AREA")
        data.energy = power
        data.size = size
        obj = bpy.data.objects.new(name, data)
        obj.location = loc
        bpy.context.collection.objects.link(obj)
        c = obj.constraints.new("TRACK_TO")
        empty = bpy.data.objects.new(f"{name}_target", None)
        empty.location = target
        bpy.context.collection.objects.link(empty)
        c.target = empty
        c.track_axis = "TRACK_NEGATIVE_Z"
        c.up_axis = "UP_Y"


def _set_engine(scene, engine):
    names = {
        "cycles": ["CYCLES"],
        "eevee": ["BLENDER_EEVEE_NEXT", "BLENDER_EEVEE"],
        "workbench": ["BLENDER_WORKBENCH"],
    }[engine]
    for n in names:
        try:
            scene.render.engine = n
            return n
        except TypeError:
            continue
    raise RuntimeError(f"render engine {engine} not available")


def setup_render(width, height, fps, frames, quality="preview", engine="cycles", gpu=False):
    scene = bpy.context.scene
    _set_engine(scene, engine)
    scene.render.resolution_x = width
    scene.render.resolution_y = height
    scene.render.resolution_percentage = 100
    scene.render.fps = fps
    scene.frame_start = 1
    scene.frame_end = max(1, frames)
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.view_settings.view_transform = "AgX"
    scene.render.film_transparent = False

    if scene.render.engine == "CYCLES":
        cyc = scene.cycles
        cyc.samples = {"preview": 12, "draft": 48, "lean": 16, "final": 160}[quality]
        cyc.use_denoising = True
        cyc.max_bounces = 6 if quality == "final" else 3
        cyc.use_adaptive_sampling = True
        if gpu:
            prefs = bpy.context.preferences.addons["cycles"].preferences
            for backend in ("OPTIX", "CUDA", "HIP", "METAL", "ONEAPI"):
                try:
                    prefs.compute_device_type = backend
                    prefs.get_devices()
                    if any(d.type == backend for d in prefs.devices):
                        for d in prefs.devices:
                            d.use = d.type == backend
                        cyc.device = "GPU"
                        break
                except TypeError:
                    continue
    elif scene.render.engine == "BLENDER_WORKBENCH":
        # Animatic look: flat-lit material colours, renders in about a second per frame on CPU.
        sh = scene.display.shading
        sh.light = "STUDIO"
        sh.color_type = "MATERIAL"
        sh.show_specular_highlight = True
        sh.show_cavity = True
        sh.cavity_type = "WORLD"
        scene.display.render_aa = "8"
    elif scene.render.engine.startswith("BLENDER_EEVEE"):
        scene.eevee.taa_render_samples = {"preview": 16, "draft": 32, "lean": 32, "final": 96}[quality]
