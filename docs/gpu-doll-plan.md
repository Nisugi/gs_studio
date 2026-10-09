# The injury doll on the GPU: modelled bodies as the production figure

Decided 2026-10-07. The creature field and the doll target a desktop with
a GPU; Android and a no-GPU fallback are no longer design constraints
(CLAUDE.md's "Android-safe" rule is a layering rule now, not a platform
one). Every playable race is getting a modelled body (`image-gen/
mesh_pack.sh`: a mesh from four painted views, a splat cloud trained from
pictures of it, Brush), so the doll's figure becomes that body, posed and
drawn on the GPU, and the ring puppet's smooth skin becomes the figure of
forms that have no body yet.

## Why

Measured 2026-10-07 (`a_pose_weighs_what_its_pixels_do`): a pose of the
GPU-drawn smooth skin at the 360 px step is 3.5 MB of CPU-built corners,
rebuilt for every breath, facing, injury and status, and the pose cache
reached the better part of a gigabyte in Hydra. A modelled body's cloud is
uploaded once and posed by bone on the GPU (`cloud_gpu.wgsl`); a pose of
it is bones plus a draw order, ~4 bytes a splat. The cloud is also the
sharper figure: it is the painted body, not a measured approximation of a
silhouette.

## The stack

1. **The body is the figure.** A form with a body in the pool is drawn as
   its trained splat cloud (`body_cloud = 1`) by default; its mesh is the
   fallback when no cloud was trained; splats made from the mesh are a
   studio choice only (a 2.4M-triangle mesh makes a storage buffer past
   wgpu's default 128 MiB binding limit).
2. **Bodies travel through the pool**, like puppets: Save installs
   `<data>/meshes/<form>/` (the largest size: `body.obj`, `color.png`,
   `normal.png`, `weights.txt`, `splat.spz` or `.ply`) into
   `<pool>/global/bodies/<form>/`, and the form's own `puppet.toml` into
   `global/puppets/` as before. `gs_field_egui` finds a form's body there
   by itself (`creature_puppet::body_dir`), so Hydra needs no studio code:
   `set_body_dir` stays as an override.
3. **The doll decides for itself.** `injury_doll::view::key_for` turns `modelled`
   on when the form has a body, and the rule that makes a hurt, scarred,
   coated or status-bearing figure a LAYERED one (`PuppetKey::with_layers`,
   studio's `DollWindow::key` until now) lives in `doll_view`, so Hydra
   and the studio draw the same doll.
4. **Splats by size.** A cloud is thinned by importance (opacity times
   footprint) into tiers, and the tier is picked by the key's size step:
   the whole cloud at 240 px and up, a third at 110 and 160, an eighth at
   76 and under. Thinning is at load, not pack time: the `.spz` files are
   small (0.5-0.7 MB a race, harmonics off).
5. **Larger size steps for bodies** (520, 760) so the doll window no
   longer magnifies a 360 px build; the smooth skin keeps its 360 cap,
   since its pose is pixels.
6. **One lighting pass.** The body draws albedo, lit base and geometry
   into the v2 atlas as every figure does; the lamp pass lights it. The
   marks sheet's wetness and glow, dropped on the cloud path until now,
   reach the GPU (glow per splat; wetness in the geo normal's length, as
   the skin path encodes it).

## Status (2026-10-07)

Done in code, tested on the CPU (`cargo test`), not yet looked at on the
GPU: items 1-6 above. Item 4's tiers are picked by size step; item 5's
larger steps are for bodies only (the smooth skin stays capped at 360,
where a pose is pixels). Item 6 also gave a hurt body's fitted
Gaussians the mesh's facing, which they lacked. A form's body is
installed into the pool only when the form brings its own puppet, so the
humanoid's `ranger` body stays a studio choice and Hydra's `humanoid`
doll is unchanged until it names a race's form.

Later the same day: webbed and frozen on a body (strands, threads and an
ice coat, all see-through films), and the doll's own roster: the pool's
bodies (`injury_doll::roster`), races then extras, with a grid figure
as the first extra. The creature forms (`humanoid`...) are the field's.

## Known gaps left open (in order of value)

- **A hit stalled the frame (fixed 2026-10-07).** Each of a fresh hit's
  ~80 steps rebuilt and rebound the whole hurt cloud on the UI thread.
  Now: a dot is bound without a search (a worn trained splat as that
  splat is bound, a dot made on an anchor to the anchor's vertex, found
  once a body); only the hit part is rebuilt a step, the rest made once a
  hit and kept; and all of it is made on a worker, the last cloud standing
  in. Measured (release, half-elf male thinned to 76k triangles): a step
  13-24 ms, was 75-130 (550 on the 2.48M-triangle mesh); a first wound
  392 ms, off the frame. The grid figure (`made_splats = 300000` in its
  doll.toml, was 800k): a first wound 1.0 s, a step 43-68 ms.
  Then the arrival moved to the GPU: one build a hit (the hit part at each
  of `layers::ARRIVAL_AGES`, blended by the hit's age in
  `cloud_gpu.wgsl`, and what it throws as particles moved there by the
  same formula as `layers::Particle::at`), played from when the build is
  in. Measured: 91-111 ms once a hit on the half-elf male (18 MB upload
  once), 0.4-0.7 s on the grid figure (88 MB); no CPU work frame to
  frame. Seen on the GPU (2026-10-07 shots): drops in flight mid-hit,
  landed round the feet after; frozen ice see-through; web strands.
  The rest of the body's pits go with it to the hit part's frames, so a
  chest hit while the belly is open leaves no chest skin standing in the
  belly's wound (`a_hit_part_is_cleared_out_of_its_neighbours_open_wound`).
- A master body is big in memory and slow to load (half_elf_male: 2.4M
  triangles, a 282 MB OBJ, parsed on a worker since 2026-10-07 while the
  smooth skin stands in): the pool may want a smaller size than the
  master, or a binary mesh format.
- Held props do not draw on an unmarked body (the layered build samples
  them at rest). Proper weapon models are wanted anyway (the procedural
  props of `data/props/weapons.toml` are stand-ins).
- The contour and the boss outline are not drawn round a cloud.
- The cloud's counting sort is single-threaded and allocates per pose:
  fine to ~100k splats; a GPU sort if clouds grow past that.
- The stage (`creature_field.rs`) never submits `Figure::Body`: field
  creatures still draw the smooth skin.
- Hydra still draws `humanoid` with a skin picker: it should list
  `injury_doll::roster::roster()` and paint the chosen figure's form.
