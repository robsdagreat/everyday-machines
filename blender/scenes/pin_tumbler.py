"""Video 001: the pin tumbler lock.

Geometry is real-mechanism logic, not just pictures: every pin height is computed
from the key's cut profile at each frame, so the animation is physically correct
for any bitting you put in params (try a different key and re-render).

Coordinates: X = along the key (front face of the lock at x=0, key enters from -X),
Z = up, section plane at y=0 (front halves are on -Y, the camera side).
"""
import math

import bpy

from em import anim, parts, style

# ---------------------------------------------------------------- dimensions
R = 0.6                 # plug radius; the top of the plug (z=R) is the shear line
L = 3.0                 # plug length
SHELL_R = 0.95          # outer radius of the housing around the plug
PIN_X = [0.55, 1.05, 1.55, 2.05, 2.55]
PIN_R = 0.11
CLEAR = 0.012           # running clearance so parts never z-fight
CHAMBER_TOP = R + 1.35
HOUSING_TOP = R + 1.5
KEYWAY = (-0.1, 0.1, -0.56, 0.14)   # y0, y1, z0, z1
Z_REST = -0.32          # where a key pin bottoms out with no key in the lock
DRIVER_LEN = 0.42
SPRING_LEN = 1.0        # modelled length; scaled to fit the chamber every frame

KEY_TOP = 0.08          # height of the blade between cuts
KEY_BOTTOM = -0.5
KEY_THICK = 0.16
BLADE_LEN = PIN_X[-1] + 0.3
CUT_SLOPE = 1.2         # steepness of the V between cuts
CUT_FLAT = 0.1          # flat at the bottom of each cut

GOOD_CUTS = [-0.15, 0.05, -0.25, 0.0, -0.1]
WRONG_CUTS = [-0.05, 0.05, -0.12, -0.25, -0.1]
KEY_PIN_LEN = [R - b for b in GOOD_CUTS]   # the lock is "keyed" to GOOD_CUTS

KEY_OUT = -BLADE_LEN - 0.6   # key x-offset when fully withdrawn


def key_top(xk, cuts):
    """Height of the key's top edge at blade coordinate xk (0 = shoulder)."""
    if xk < 0 or xk > BLADE_LEN:
        return None
    z = KEY_TOP
    for xi, b in zip(PIN_X, cuts):
        z = min(z, b + CUT_SLOPE * max(0.0, abs(xk - xi) - CUT_FLAT / 2))
    z = min(z, KEY_TOP - CUT_SLOPE * max(0.0, xk - (BLADE_LEN - 0.25)))  # tip ramp
    return z


# ---------------------------------------------------------------- build

def _key(name, cuts, mat):
    pts = [(-0.25, KEY_BOTTOM), (BLADE_LEN - 0.12, KEY_BOTTOM), (BLADE_LEN, KEY_BOTTOM + 0.12)]
    n = 240
    for i in range(n + 1):
        xk = BLADE_LEN * (1 - i / n)
        pts.append((xk, key_top(xk, cuts)))
    pts.append((-0.25, KEY_TOP))
    blade = parts.extrude_profile(name, pts, KEY_THICK)
    style.assign(blade, mat)
    bow = parts.cylinder(f"{name}_bow", 0.55, 0.2, (-0.75, 0, -0.2), axis="Y")
    parts.boolean(bow, parts.cylinder("ring", 0.14, 1, (-1.0, 0, -0.2), axis="Y"))
    style.assign(bow, mat)
    parts.bevel(bow, 0.03)
    bow.parent = blade
    bow.location = (-0.75, 0, -0.2)
    blade.location = (KEY_OUT, 0, 0)
    return blade


def build(params):
    style.setup_lights(target=(1.5, 0, 0.6))
    good = params.get("good_cuts", GOOD_CUTS)
    wrong = params.get("wrong_cuts", WRONG_CUTS)
    pin_len = [R - b for b in good]

    # Housing: round shell around the plug + a tower that carries the pin chambers.
    housing = parts.cylinder("housing", SHELL_R, L, (L / 2, 0, 0), axis="X")
    parts.boolean(housing, parts.box("tower", 0, L, -0.55, 0.55, 0, HOUSING_TOP), op="UNION")
    cutters = [parts.cylinder("bore", R + CLEAR, L + 1, (L / 2, 0, 0), axis="X")]
    for x in PIN_X:
        z0, z1 = R - 0.1, CHAMBER_TOP
        cutters.append(parts.cylinder(f"chamber_{x}", PIN_R + CLEAR, z1 - z0, (x, 0, (z0 + z1) / 2)))
    parts.boolean(housing, cutters)
    style.assign(housing, style.metal("housing", 0.45))

    # Plug: the part that turns. Keyway cut through the bottom, pin holes through the top.
    plug = parts.cylinder("plug", R, L + 0.12, ((L - 0.12) / 2, 0, 0), axis="X")
    y0, y1, kz0, kz1 = KEYWAY
    cutters = [parts.box("keyway", -1, L + 1, y0, y1, kz0, kz1)]
    for x in PIN_X:
        cutters.append(parts.cylinder(f"hole_{x}", PIN_R + CLEAR, R + 0.3, (x, 0, R / 2)))
    parts.boolean(plug, cutters)
    style.assign(plug, style.metal("plug", 0.3))

    housing_back, housing_front = parts.split_section(housing)
    plug_back, plug_front = parts.split_section(plug)

    pivot = bpy.data.objects.new("plug_pivot", None)   # rotation axis = X axis
    bpy.context.collection.objects.link(pivot)
    for o in (plug_back, plug_front):
        o.parent = pivot

    key_pins, driver_pins, springs = [], [], []
    for i, x in enumerate(PIN_X):
        kp = parts.cylinder(f"key_pin_{i}", PIN_R, pin_len[i], (x, 0, 0))
        style.assign(parts.bevel(kp, 0.035), style.metal("key_pin", 0.3))
        kp.parent = pivot
        dp = parts.cylinder(f"driver_pin_{i}", PIN_R, DRIVER_LEN, (x, 0, 0))
        style.assign(parts.bevel(dp, 0.02), style.metal("driver_pin", 0.3))
        sp = parts.spring(f"spring_{i}", PIN_R * 0.8, 0.018, 7, SPRING_LEN, (x, 0, CHAMBER_TOP))
        sp.data.materials.append(style.metal("spring", 0.25))
        key_pins.append(kp)
        driver_pins.append(dp)
        springs.append(sp)

    key_good = _key("key", good, style.metal("key", 0.3))
    key_wrong = _key("key_wrong", wrong, style.metal("key", 0.3))
    for k in (key_good, key_wrong):
        k.parent = pivot

    shear_y = -PIN_R - 0.03
    shear = anim.rod("shear_line", (-0.3, shear_y, R), (L + 0.3, shear_y, R), 0.014, style.glow("highlight", 4))
    shear_ok = anim.rod("shear_ok", (-0.3, shear_y, R), (L + 0.3, shear_y, R), 0.016, style.glow("ok", 5))
    for o in (shear, shear_ok):
        o.scale = (0, 0, 0)

    ctx = dict(housing_back=housing_back, housing_front=housing_front, plug_back=plug_back,
               plug_front=plug_front, pivot=pivot, key_pins=key_pins, driver_pins=driver_pins,
               springs=springs, key=key_good, key_wrong=key_wrong, shear=shear, shear_ok=shear_ok,
               good=good, wrong=wrong, pin_len=pin_len)
    set_pins(ctx, frame=None, key_x=None, cuts=None)
    return ctx


# ---------------------------------------------------------------- mechanism

def pin_state(ctx, key_x, cuts):
    """Return [(key_pin_center_z, driver_center_z, spring_scale)] for a key at offset key_x."""
    out = []
    for i, x in enumerate(PIN_X):
        bottom = Z_REST
        if key_x is not None:
            top = key_top(x - key_x, cuts)
            if top is not None:
                bottom = max(Z_REST, top)
        kp_top = bottom + ctx["pin_len"][i]
        dp_top = kp_top + DRIVER_LEN
        out.append((bottom + ctx["pin_len"][i] / 2, kp_top + DRIVER_LEN / 2,
                    (CHAMBER_TOP - dp_top) / SPRING_LEN))
    return out


def set_pins(ctx, frame, key_x, cuts, interp="LINEAR"):
    for i, (kz, dz, s) in enumerate(pin_state(ctx, key_x, cuts)):
        x = PIN_X[i]
        kp, dp, sp = ctx["key_pins"][i], ctx["driver_pins"][i], ctx["springs"][i]
        kp.location = (x, 0, kz)
        dp.location = (x, 0, dz)
        sp.scale = (1, 1, s)
        if frame is not None:
            anim.loc(kp, frame, interp=interp)
            anim.loc(dp, frame, interp=interp)
            anim.scale(sp, frame, interp=interp)


def insert_key(ctx, key, cuts, f0, f1, start=KEY_OUT, end=0.0):
    """Slide `key` from x=start to x=end between frames f0..f1, pins riding the cuts."""
    for f in range(1, f1 + 1):
        u = anim.smoothstep((f - f0) / max(1, f1 - f0)) if f >= f0 else 0.0
        kx = start + (end - start) * u
        key.location = (kx, 0, 0)
        anim.loc(key, f, interp="LINEAR")
        set_pins(ctx, f, kx, cuts)


def section_view(ctx, cam, visible_front=False):
    for o in (ctx["housing_front"], ctx["plug_front"]):
        o.hide_render = not visible_front
    anim.set_boom(cam, 10.2, 6, -90)


def hide(*objs):
    """Hide objects and their children (hide_render does not propagate)."""
    for o in objs:
        o.hide_render = True
        hide(*o.children)


# ---------------------------------------------------------------- shots

def shot_exterior(ctx, t):
    """Hero shot: the lock as you know it, slow orbit, title."""
    cam, pivot = anim.camera_rig((1.4, 0, 0.7), 12.0, 14, -150, lens=50)
    hide(ctx["key"], ctx["key_wrong"])
    anim.rot(pivot, 1, (0, 0, 0))
    anim.rot(pivot, t.frames, (0, 0, math.radians(28)))
    title = anim.label("PIN TUMBLER LOCK", (1.4, -0.6, HOUSING_TOP + 0.55), size=0.32, color="label")
    anim.pop_in(title, t.at(0.15), 10)


def shot_reveal(ctx, t):
    """Slice the lock in half and name the parts."""
    cam, pivot = anim.camera_rig((1.3, 0, 0.55), 12.0, 14, -150, lens=46)
    hide(ctx["key"], ctx["key_wrong"])
    a, b = t.span(0.05, 0.3)
    for o in (ctx["housing_front"], ctx["plug_front"]):
        anim.loc(o, a, (0, 0, 0))
        anim.loc(o, b, (0, -3.0, 0))
        anim.pop_out(o, b - 4, 6)
    anim.boom(cam, a, 12.0, 14, -150)
    anim.boom(cam, t.at(0.35), 10.2, 6, -90)

    y = -0.45
    labels = [
        ("housing", (L + 0.75, y, 1.65), 0.40),
        ("springs", (-0.75, y, 1.65), 0.48),
        ("driver pins", (-0.95, y, 1.0), 0.56),
        ("key pins", (-0.85, y, 0.3), 0.64),
        ("plug", (L + 0.6, y, 0.35), 0.72),
        ("keyway", (L + 0.75, y, -0.3), 0.80),
    ]
    for text, pos, when in labels:
        anim.pop_in(anim.label(text, pos, size=0.2), t.at(when), 8)


def shot_locked(ctx, t):
    """Why it won't turn: driver pins straddle the shear line."""
    cam, _ = anim.camera_rig((1.3, 0, 0.55), 10.2, 6, -90, lens=46)
    section_view(ctx, cam)
    hide(ctx["key"], ctx["key_wrong"])
    anim.pop_in(ctx["shear"], t.at(0.1), 10)
    anim.pop_in(anim.label("shear line", (-0.9, -0.45, R), size=0.2, color="highlight"), t.at(0.15), 8)
    a = t.at(0.4)
    for dp in ctx["driver_pins"]:   # pulse the culprits
        anim.scale(dp, a, 1.0)
        anim.scale(dp, a + 6, 1.18)
        anim.scale(dp, a + 14, 1.0)
    anim.wiggle_rotation(ctx["pivot"], "X", t.at(0.62), max(12, t.at(0.9) - t.at(0.62)), degrees=-3)
    anim.pop_in(anim.label("blocked", (1.5, -0.45, -1.25), size=0.24, color="bad"), t.at(0.65), 8)


def shot_wrong_key(ctx, t):
    """A wrong key lifts the pins to the wrong heights."""
    cam, pivot = anim.camera_rig((-0.7, 0, 0.55), 11.0, 6, -90, lens=46)
    section_view(ctx, cam)
    anim.set_boom(cam, 11.0, 6, -90)
    hide(ctx["key"])
    ctx["shear"].scale = (1, 1, 1)
    a, b = t.span(0.08, 0.55)
    anim.loc(pivot, a, (-0.7, 0, 0.55))
    anim.loc(pivot, b, (1.3, 0, 0.55))
    anim.boom(cam, a, 11.0, 6, -90)
    anim.boom(cam, b, 10.2, 6, -90)
    insert_key(ctx, ctx["key_wrong"], ctx["wrong"], a, b)
    anim.wiggle_rotation(ctx["pivot"], "X", t.at(0.66), max(12, t.at(0.92) - t.at(0.66)), degrees=-3)
    anim.pop_in(anim.label("wrong key: gaps miss the line", (1.5, -0.45, -1.25), size=0.2, color="bad"),
                t.at(0.6), 8)


def shot_right_key(ctx, t):
    """The right key puts every gap exactly on the shear line."""
    cam, pivot = anim.camera_rig((-0.7, 0, 0.55), 11.0, 6, -90, lens=46)
    section_view(ctx, cam)
    anim.set_boom(cam, 11.0, 6, -90)
    hide(ctx["key_wrong"])
    ctx["shear"].scale = (1, 1, 1)
    a, b = t.span(0.08, 0.55)
    anim.loc(pivot, a, (-0.7, 0, 0.55))
    anim.loc(pivot, b, (1.3, 0, 0.55))
    anim.boom(cam, a, 11.0, 6, -90)
    anim.boom(cam, b, 10.2, 6, -90)
    insert_key(ctx, ctx["key"], ctx["good"], a, b)
    c = t.at(0.62)
    anim.pop_out(ctx["shear"], c, 4)
    anim.pop_in(ctx["shear_ok"], c, 8)
    anim.pop_in(anim.label("every gap on the line", (1.5, -0.45, -1.25), size=0.22, color="ok"), c + 4, 8)


def shot_turn(ctx, t):
    """With the gaps aligned, the plug (and key) can rotate and drive the cam."""
    cam, pivot = anim.camera_rig((0.8, 0, 0.7), 10.0, 16, -140, lens=50)
    hide(ctx["key_wrong"], ctx["shear"], ctx["shear_ok"])
    ctx["key"].location = (0, 0, 0)
    set_pins(ctx, None, 0.0, ctx["good"])
    cam_bar = parts.box("cam", L + 0.02, L + 0.18, -0.12, 0.12, -R - 0.35, 0)
    style.assign(cam_bar, style.metal("highlight", 0.35))
    cam_bar.parent = ctx["pivot"]
    a, b = t.span(0.2, 0.7)
    anim.rot(ctx["pivot"], a, (0, 0, 0))
    anim.rot(ctx["pivot"], b, (math.radians(-90), 0, 0))
    anim.rot(pivot, 1, (0, 0, 0))
    anim.rot(pivot, t.frames, (0, 0, math.radians(-25)))
    anim.pop_in(anim.label("plug turns", (1.4, -0.9, HOUSING_TOP + 0.25), size=0.26), t.at(0.3), 8)


SHOTS = {
    "exterior": shot_exterior,
    "reveal": shot_reveal,
    "locked": shot_locked,
    "wrong_key": shot_wrong_key,
    "right_key": shot_right_key,
    "turn": shot_turn,
}
