"""How much each bone of a puppet moves each vertex of a mesh, by Blender's
automatic weights (bone heat: a bone's pull spreads through the body, so an
armpit goes with the arm and not the ribs it lies beside). Run in Blender:

    blender -b -P tools/mesh_weights.py -- <mesh dir> [--still clavicle,...]

`<mesh dir>` holds body.obj (y up, facing +z, standing as the rig was drawn)
and bones.txt (the rig's bones in the puppet's rest space: the `mesh_pose`
example writes it). Writes weights.txt there, which `gs_puppet::mesh::
load_weights` reads: a `bone <name>` line per bone, then a `w <bone>:<weight>
...` line per vertex of body.obj, in its order.

A bone of one side of the body keeps to its side. A biped's two collarbones
both start at the middle of the chest, so heat gives them the whole upper
chest between them; raising the arms turns them opposite ways and tears it.
So a bone that runs from the middle out to one side (a collarbone) only takes
hold toward its outer end -- nothing at the middle, all of its heat weight
from most of the way out -- and any other bone of one side (an arm, a leg)
loses what it has across the middle. What they lose goes to the nearest
ancestor on the centre line (the torso). `--still <prefix,...>` names bones
that take no vertices at all.

The mesh is cut along its sheet's seams (a vertex per sheet place), and heat
does not cross a cut: the weights are worked out on a welded copy and each
vertex takes those of the welded vertex where it is.
"""
import os, sys

import bpy
from mathutils import Vector, kdtree

args = sys.argv[sys.argv.index("--") + 1:]
folder = args[0]
still = tuple(args[args.index("--still") + 1].split(",")) if "--still" in args else ()
height, bones = 0.0, []
for line in open(os.path.join(folder, "bones.txt")):
    t = line.split()
    if t and t[0] == "height":
        height = float(t[1])
    elif t and t[0] == "bone":
        bones.append((t[1], t[2], [float(v) for v in t[3:6]], [float(v) for v in t[6:9]]))

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.wm.obj_import(filepath=os.path.join(folder, "body.obj"), forward_axis="NEGATIVE_Z", up_axis="Y")
obj = bpy.context.selected_objects[0]
bpy.context.view_layer.objects.active = obj
bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
count = sum(1 for line in open(os.path.join(folder, "body.obj")) if line.startswith("v "))
verts = obj.data.vertices
if len(verts) != count:
    raise SystemExit(f"body.obj has {count} vertices, Blender read {len(verts)}: the orders would not match")

# as `Mesh::fit_to_height`: the puppet's height, feet on the ground, centred
# (here z is up and the figure faces -y: the puppet's (x, y, z) is (x, -z, y))
lo = Vector((min(v.co[k] for v in verts) for k in range(3)))
hi = Vector((max(v.co[k] for v in verts) for k in range(3)))
s = height / (hi.z - lo.z)
centre = Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, lo.z))
for v in verts:
    v.co = (v.co - centre) * s

weld = obj.copy()
weld.data = obj.data.copy()
bpy.context.scene.collection.objects.link(weld)
bpy.ops.object.select_all(action="DESELECT")
weld.select_set(True)
bpy.context.view_layer.objects.active = weld
bpy.ops.object.mode_set(mode="EDIT")
bpy.ops.mesh.select_all(action="SELECT")
bpy.ops.mesh.remove_doubles(threshold=height * 2e-4)
bpy.ops.mesh.normals_make_consistent(inside=False)
bpy.ops.object.mode_set(mode="OBJECT")
print(f"{len(verts)} vertices, {len(weld.data.vertices)} welded", flush=True)

arm = bpy.data.objects.new("rig", bpy.data.armatures.new("rig"))
bpy.context.scene.collection.objects.link(arm)
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="EDIT")
to_blender = lambda p: Vector((p[0], -p[2], p[1]))
made = {}
for name, parent, head, tail in bones:
    b = arm.data.edit_bones.new(name)
    b.head, b.tail = to_blender(head), to_blender(tail)
    if (b.tail - b.head).length < height * 0.004:   # Blender drops a bone with no length
        b.tail = b.head + Vector((0, 0, height * 0.004))
    made[name] = b
for name, parent, _, _ in bones:
    if parent in made:
        made[name].parent = made[parent]
bpy.ops.object.mode_set(mode="OBJECT")
for bone in arm.data.bones:
    if still and bone.name.startswith(still):
        bone.use_deform = False

bpy.ops.object.select_all(action="DESELECT")
weld.select_set(True)
arm.select_set(True)
bpy.context.view_layer.objects.active = arm
bpy.ops.object.parent_set(type="ARMATURE_AUTO")

names = [b[0] for b in bones]
# each bone's side (-1, 0 the centre line, 1), where along x its hold starts
# and where it is whole, and the centre-line ancestor that takes what it loses
MID = height * 0.04
by_name = {b[0]: b for b in bones}
def sided(name):
    _, parent, head, tail = by_name[name]
    if abs(tail[0]) < MID and abs(head[0]) < MID:
        return None
    side = 1.0 if (tail[0] if abs(tail[0]) >= abs(head[0]) else head[0]) > 0 else -1.0
    if abs(head[0]) < MID:                          # from the middle outward: a collarbone
        reach = abs(tail[0])
        ramp = (0.15 * reach, 0.7 * reach)
    else:
        ramp = (-0.02 * height, 0.02 * height)      # across the middle: gone
    heir = parent
    while heir in by_name and sided(heir) is not None:
        heir = by_name[heir][1]
    return side, ramp, names.index(heir) if heir in by_name else None
SIDED = {names.index(n): sided(n) for n in names if sided(n) is not None}
def smoothstep(a, b, x):
    t = min(1.0, max(0.0, (x - a) / (b - a)))
    return t * t * (3 - 2 * t)
def keep_to_side(shares, x):
    out = {}
    for bone, w in shares:
        rule = SIDED.get(bone)
        if rule:
            side, ramp, heir = rule
            kept = w * smoothstep(ramp[0], ramp[1], side * x)
            if heir is not None:
                out[heir] = out.get(heir, 0.0) + w - kept
            w = kept
        out[bone] = out.get(bone, 0.0) + w
    return [(b, w) for b, w in out.items() if w > 1e-4]
group_bone = {g.index: names.index(g.name) for g in weld.vertex_groups if g.name in names}
# heat can fail outright ("failed to find solution for one or more bones": a mesh it cannot solve, every vertex left
# bare). Then each welded vertex takes the two nearest bone segments, by the inverse fourth power of its distance
# to each, and the side rules below still apply.
held = sum(1 for v in weld.data.vertices if any(g.group in group_bone and g.weight > 1e-4 for g in v.groups))
if held < 0.5 * len(weld.data.vertices):
    print(f"heat held {held} of {len(weld.data.vertices)} welded vertices: binding by nearest bone segments instead", flush=True)
    segs = []
    for name, parent, head, tail in bones:
        if still and name.startswith(still):
            continue
        a, b = to_blender(head), to_blender(tail)
        if (b - a).length < height * 0.004:
            b = a + Vector((0, 0, height * 0.004))
        segs.append((names.index(name), a, b))
    for g in list(weld.vertex_groups):
        weld.vertex_groups.remove(g)
    groups = {i: weld.vertex_groups.new(name=names[i]) for i, _, _ in segs}
    group_bone = {g.index: names.index(g.name) for g in weld.vertex_groups}
    for v in weld.data.vertices:
        ds = []
        for i, a, b in segs:
            ab = b - a
            t = min(1.0, max(0.0, (v.co - a).dot(ab) / ab.length_squared))
            ds.append(((v.co - (a + ab * t)).length, i))
        ds.sort()
        near = ds[:2]
        ws = [1.0 / max(d, height * 1e-3) ** 4 for d, _ in near]
        tot = sum(ws)
        for (d, i), wgt in zip(near, ws):
            if wgt / tot > 1e-3:
                groups[i].add([v.index], wgt / tot, "REPLACE")
tree = kdtree.KDTree(len(weld.data.vertices))
for v in weld.data.vertices:
    tree.insert(v.co, v.index)
tree.balance()
bare = 0
with open(os.path.join(folder, "weights.txt"), "w") as out:
    for name in names:
        out.write(f"bone {name}\n")
    for v in verts:
        _, k, _ = tree.find(v.co)
        shares = [(group_bone[g.group], g.weight) for g in weld.data.vertices[k].groups if g.group in group_bone and g.weight > 1e-4]
        shares = keep_to_side(shares, v.co.x)
        bare += not shares
        out.write("w " + " ".join(f"{b}:{w:.4f}" for b, w in shares) + "\n")
print(f"weights.txt: {len(verts)} vertices, {bare} with no bone", flush=True)
