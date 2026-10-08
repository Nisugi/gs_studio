"""A modelled body's mesh thinned for runtime (Blender, CPU, no render).

    blender -b -P tools/decimate_mesh.py -- <in body.obj> <out body_thin.obj> [<faces>]

Every doll figure is drawn as splats; a body's mesh is never shown. It only
gives each trained splat its bone shares and its facing (gs_puppet::mesh::
SplatBody::bind: the nearest vertex). The splats keep the detail, so the
mesh can be thin. Measured 2026-10-07 on half_elf_male (35.6k splats): a
2.48M-triangle mesh loads in 5.7 s and makes each fresh-hit rebuild ~550 ms;
thinned, 0.5 s and 75-130 ms, every splat still bound.

The mesh is thinned by merging the vertices that lie close together (a
cell a fraction of its height across, coarser until it is near `faces`
triangles). Not by Blender's collapse decimate: that deletes small or thin
parts outright -- half_elf_male's spliced hands and forearms went, and the
splats on them were left with nothing to bind to.

ITS PAINT IS NOT VALID: the merge runs across the sheet's seams, and the
generator's sheet islands are smaller than a thin triangle, so no remapping
can put the paint back (three ways tried, 2026-10-07). That is fine for what
it is for -- binding splats, never drawn -- so it is written as
body_thin.obj beside the full body.obj (which the image-gen tools draw, and
retraining starts from), never over it. Never thin a body whose splats are
MADE from its mesh and sheet (`draw = "made"` in its doll.toml): those read
paint through it. The grid figure is trained now (Brush on pictures of its
grid sheet, 2026-10-08), so it is thinned like any trained body.
image-gen/pack_figure.sh's thin stage (THIN=1) runs this, then
tools/mesh_weights.py for weights_thin.txt.

The OBJ goes through Blender's importer and exporter with the same axes
(-Z forward, Y up), so the body stays y up, facing +z, where it stood.
"""
import sys
import time

import bpy

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
if len(argv) < 2:
    print("usage: blender -b -P tools/decimate_mesh.py -- <in body.obj> <out body_thin.obj> [<faces>]")
    sys.exit(2)
src_path, dst_path = argv[0], argv[1]
if src_path.replace("\\", "/").rstrip("/") == dst_path.replace("\\", "/").rstrip("/"):
    print("Error: the thin mesh is never written over the full one")
    sys.exit(2)
target = int(argv[2]) if len(argv) > 2 else 40000

t = time.time()
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.wm.obj_import(filepath=src_path, forward_axis="NEGATIVE_Z", up_axis="Y")
objs = [o for o in bpy.context.selected_objects if o.type == "MESH"]
if not objs:
    print(f"Error: no mesh in {src_path}")
    sys.exit(1)
bpy.context.view_layer.objects.active = objs[0]
if len(objs) > 1:
    bpy.ops.object.join()
ob = bpy.context.view_layer.objects.active


def triangles(o):
    return sum(len(p.vertices) - 2 for p in o.data.polygons)


verts_in = len(ob.data.vertices)
tris = triangles(ob)
# Thinned by MERGING what lies close (a cell a fraction of the figure's
# height across), not by collapsing edges: a collapse deletes small or thin
# parts outright (half_elf_male's spliced hands and forearms went, and the
# splats on them with nothing left to bind to), where a merge keeps a
# vertex every cell over every part, however thin. A merge that leaves more
# than the target is run again, coarser.
lo = min(v.co.y for v in ob.data.vertices)
hi = max(v.co.y for v in ob.data.vertices)
cell = (hi - lo) * 0.0025
for _ in range(6):
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.remove_doubles(threshold=cell)
    bpy.ops.mesh.dissolve_degenerate(threshold=cell * 0.01)
    bpy.ops.object.mode_set(mode="OBJECT")
    if triangles(ob) <= target * 1.15:
        break
    cell *= 1.35
welded = verts_in - len(ob.data.vertices)
# every face turned out: a merge leaves some wound the other way, and a
# splat takes its facing from the vertex nearest it
bpy.ops.object.mode_set(mode="EDIT")
bpy.ops.mesh.select_all(action="SELECT")
bpy.ops.mesh.normals_make_consistent(inside=False)
# and no vertex left out of every face: an OBJ importer drops one (Blender's
# does), so the weights worked out on the file's vertices (mesh_weights.py)
# would not line up with them -- half_elf_female's merge left two
bpy.ops.mesh.select_all(action="SELECT")
bpy.ops.mesh.delete_loose(use_verts=True, use_edges=True, use_faces=False)
bpy.ops.object.mode_set(mode="OBJECT")
for poly in ob.data.polygons:
    poly.use_smooth = True
bpy.ops.wm.obj_export(
    filepath=dst_path,
    export_selected_objects=True,
    forward_axis="NEGATIVE_Z",
    up_axis="Y",
    export_uv=True,
    export_normals=True,
    export_materials=False,
    export_triangulated_mesh=True,
)
print(f"decimated {tris} -> {triangles(ob)} triangles, {verts_in} -> {len(ob.data.vertices)} vertices (merged within {cell:.5f}; for binding splats, its paint not kept) in {time.time() - t:.1f} s: {dst_path}")
