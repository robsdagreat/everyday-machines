"""Procedural geometry for mechanisms.

Everything is built from code so parts are exact, reusable and parameterised:
a pin stack, a spring or a gear is one function call with real dimensions.
Units are arbitrary "scene units"; keep one mechanism at roughly 1-5 units across
so the lighting rig and camera defaults in style/anim fit without tweaking.
"""
import math

import bmesh
import bpy
from mathutils import Matrix, Vector

AXIS_ROT = {
    "Z": Matrix.Identity(4),
    "X": Matrix.Rotation(math.pi / 2, 4, "Y"),
    "Y": Matrix.Rotation(-math.pi / 2, 4, "X"),
}


def _link(obj, collection=None):
    (collection or bpy.context.collection).objects.link(obj)
    return obj


def mesh_object(name, bm, location=(0, 0, 0), collection=None):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    obj = bpy.data.objects.new(name, me)
    obj.location = location
    return _link(obj, collection)


def cylinder(name, radius, depth, location=(0, 0, 0), axis="Z", verts=64, smooth=True):
    """Cylinder centred on `location`, its length along `axis`."""
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, segments=verts, radius1=radius, radius2=radius, depth=depth)
    bmesh.ops.transform(bm, matrix=AXIS_ROT[axis], verts=bm.verts)
    if smooth:
        for f in bm.faces:
            f.smooth = len(f.verts) == 4  # smooth the sides, keep caps flat
        for e in bm.edges:  # crisp rims: mark side/cap boundary sharp
            if len({len(f.verts) == 4 for f in e.link_faces}) > 1:
                e.smooth = False
    return mesh_object(name, bm, location)


def box(name, x0, x1, y0, y1, z0, z1):
    """Axis-aligned box from min/max corners (easier to reason about than centre+size)."""
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=(x1 - x0, y1 - y0, z1 - z0), verts=bm.verts)
    return mesh_object(name, bm, ((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2))


def apply_modifiers(obj):
    """Bake the modifier stack into the mesh without relying on operator context."""
    dg = bpy.context.evaluated_depsgraph_get()
    new_mesh = bpy.data.meshes.new_from_object(obj.evaluated_get(dg))
    old = obj.data
    obj.modifiers.clear()
    obj.data = new_mesh
    new_mesh.name = obj.name
    bpy.data.meshes.remove(old)
    return obj


def boolean(target, cutters, op="DIFFERENCE", keep_cutters=False):
    """Apply one boolean against any number of cutter objects."""
    cutters = cutters if isinstance(cutters, (list, tuple)) else [cutters]
    coll = bpy.data.collections.new(f"{target.name}_cutters")
    bpy.context.scene.collection.children.link(coll)
    for c in cutters:
        for uc in list(c.users_collection):
            uc.objects.unlink(c)
        coll.objects.link(c)
    mod = target.modifiers.new("bool", "BOOLEAN")
    mod.operation = op
    mod.solver = "EXACT"
    if len(cutters) == 1:
        mod.operand_type = "OBJECT"
        mod.object = cutters[0]
    else:
        mod.operand_type = "COLLECTION"
        mod.collection = coll
    coll.hide_render = True
    apply_modifiers(target)
    if not keep_cutters:
        for c in cutters:
            bpy.data.objects.remove(c, do_unlink=True)
        bpy.data.collections.remove(coll)
    return target


def duplicate(obj, name):
    new = obj.copy()
    new.data = obj.data.copy()
    new.name = name
    return _link(new)


def split_section(obj, plane_y=0.0, extent=50.0):
    """Cut an object along the plane y=plane_y into (back, front) halves.

    The classic cutaway: hide or slide away the front half to see inside.
    """
    front = duplicate(obj, f"{obj.name}_front")
    back = obj
    back.name = f"{obj.name}_back"
    boolean(back, box("cut_b", -extent, extent, plane_y, extent, -extent, extent), op="INTERSECT")
    boolean(front, box("cut_f", -extent, extent, -extent, plane_y, -extent, extent), op="INTERSECT")
    return back, front


def bevel(obj, width=0.015, segments=3):
    mod = obj.modifiers.new("bevel", "BEVEL")
    mod.width = width
    mod.segments = segments
    mod.limit_method = "ANGLE"
    return obj


def extrude_profile(name, points_xz, thickness, location=(0, 0, 0)):
    """Extrude a 2D outline drawn in the XZ plane by `thickness` along Y (centred).

    Good for keys, levers, cams and anything that is a flat cut-out.
    """
    bm = bmesh.new()
    verts = [bm.verts.new((x, -thickness / 2, z)) for x, z in points_xz]
    face = bm.faces.new(verts)
    res = bmesh.ops.extrude_face_region(bm, geom=[face])
    moved = [g for g in res["geom"] if isinstance(g, bmesh.types.BMVert)]
    bmesh.ops.translate(bm, vec=(0, thickness, 0), verts=moved)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return mesh_object(name, bm, location)


def spring(name, radius, wire, turns, length, location=(0, 0, 0), segments_per_turn=24):
    """Helical compression spring hanging DOWN from its origin.

    The origin is the fixed top end, so compressing it is just scaling Z:
    obj.scale.z = current_length / length.
    """
    curve = bpy.data.curves.new(name, "CURVE")
    curve.dimensions = "3D"
    curve.bevel_depth = wire
    curve.bevel_resolution = 3
    spl = curve.splines.new("POLY")
    n = int(turns * segments_per_turn)
    spl.points.add(n)
    for i in range(n + 1):
        t = i / n
        a = 2 * math.pi * turns * t
        spl.points[i].co = (radius * math.cos(a), radius * math.sin(a), -length * t, 1.0)
    obj = bpy.data.objects.new(name, curve)
    obj.location = location
    return _link(obj)


def involute_gear(name, teeth, module, thickness, bore=0.0, pressure_deg=20.0,
                  location=(0, 0, 0), axis="Z", samples=6):
    """True involute spur gear. Two gears mesh when they share `module`.

    Centre distance for a meshing pair = module * (teeth_a + teeth_b) / 2.
    """
    alpha = math.radians(pressure_deg)
    rp = module * teeth / 2          # pitch radius
    rb = rp * math.cos(alpha)        # base circle
    ra = rp + module                 # tip
    rf = max(rp - 1.25 * module, 0.1 * rp)  # root

    def inv(a):
        return math.tan(a) - a

    def theta(r):  # polar angle of the involute at radius r
        r = max(r, rb)
        return math.sqrt((r / rb) ** 2 - 1) - math.acos(rb / r)

    half_tooth = math.pi / (2 * teeth) + inv(alpha)
    r_start = max(rb, rf)
    radii = [r_start + (ra - r_start) * i / samples for i in range(samples + 1)]

    outline = []
    for k in range(teeth):
        c = 2 * math.pi * k / teeth
        right = [(r, c - (half_tooth - theta(r))) for r in radii]
        left = [(r, c + (half_tooth - theta(r))) for r in reversed(radii)]
        if rf < r_start:
            right.insert(0, (rf, right[0][1]))
            left.append((rf, left[-1][1]))
        outline += right + left

    bm = bmesh.new()
    verts = [bm.verts.new((r * math.cos(a), r * math.sin(a), -thickness / 2)) for r, a in outline]
    face = bm.faces.new(verts)
    res = bmesh.ops.extrude_face_region(bm, geom=[face])
    moved = [g for g in res["geom"] if isinstance(g, bmesh.types.BMVert)]
    bmesh.ops.translate(bm, vec=(0, 0, thickness), verts=moved)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bmesh.ops.transform(bm, matrix=AXIS_ROT[axis], verts=bm.verts)
    gear = mesh_object(name, bm, location)
    if bore > 0:
        boolean(gear, cylinder(f"{name}_bore", bore, thickness * 3, location, axis=axis))
    gear["pitch_radius"] = rp
    gear["teeth"] = teeth
    return gear


def mesh_gear_pair(gear_a, gear_b, axis="Z"):
    """Phase gear_b so its teeth sit between gear_a's teeth; returns the ratio a->b."""
    za, zb = gear_a["teeth"], gear_b["teeth"]
    idx = {"X": 0, "Y": 1, "Z": 2}[axis]
    rot = list(gear_b.rotation_euler)
    rot[idx] += math.pi / zb if zb % 2 == 0 else 0.0
    gear_b.rotation_euler = rot
    return -za / zb
