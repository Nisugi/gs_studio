# Local creature texture generator — build plan

Status: plan, 2026-09-24 (tools and versions researched that day). Audience: the engineer (human or LLM agent) who
builds the generator. Everything you need is in this file; where it says
"gs-studio will", that part is built on the gs-studio side, not by you.

## 1. What you are building

A local pipeline on a Windows 11 PC with an **AMD Radeon RX 7900 GRE (16 GB
VRAM)** that paints every GemStone IV creature three times — **front, back
and side**, full body — each painting exactly inside a silhouette that
gs-studio supplies, all in **one uniform art style**, unattended, overnight.

gs-studio (a Rust app in this repository) then projects the three paintings
onto the creature's 3D puppet ("Build sheet from paintings"). You never deal
with the 3D side, UV layouts or seams: you only produce the paintings.

Scope: about 600 bestiary creatures, drawn by 22 puppet families (humanoid,
troll, ogre, canine, feline, ...). 545 of the 627 bestiary entries have a
written description; the rest need a hand-written brief or are skipped.

### Definition of done

| Deliverable | Done when |
| --- | --- |
| ComfyUI workflow (API-format JSON, committed) + batch script (Python) | Runs a job list overnight with nobody watching; after a crash or reboot, re-running it continues where it stopped and never redoes approved work |
| Three paintings per creature | `front.png`, `back.png`, `side.png` per the contract in section 2 |
| One style for the whole set | A grid of 20 random creatures reads as one set: same brushwork, lighting, saturation |
| Review sheet per batch | One PNG grid (or HTML page) showing every result of the batch with its creature name, for approve/reject |
| Pilot | 10 humanoid-family creatures, built into skins in gs-studio, that look right from front, back and side |

## 2. The contract with gs-studio (inputs and outputs)

All paths are relative to the gs-studio **data folder** (in this repo:
`data/`; the app may be pointed elsewhere — take it as a setting).

### Inputs

| File | Written by | Content |
| --- | --- | --- |
| `textures/<family>/front_paint.png` | gs-studio: Rigs tab → Texture painting → Export templates | RGBA PNG. The creature's silhouette filled with flat grey (205, 205, 205, alpha 255) on a fully transparent background. 512 px on its long side (e.g. humanoid front: 186 × 512). |
| `textures/<family>/back_paint.png` | same | The front silhouette mirrored left-right. |
| `textures/<family>/side_paint.png` | same | The side silhouette (e.g. humanoid: 88 × 512). |
| `textures/<family>/{front,back,side}_depth.png` | same (Export templates writes them too) | **Depth guides** for ControlNet depth: the puppet's rest body seen from that view, in **exactly its template's framing at 4× its size** (humanoid front: 744 × 2048). Opaque grey: nearest surface white (255), farthest body surface grey (64), outside the silhouette black (0) — the MiDaS convention depth ControlNets expect. Hairline seams in the outline (the humanoid's side view draws its arm a sliver off the body) are closed. **Use only for approved families — today: humanoid.** |
| `textures/<family>/marks.json` | same | Where the puppet's eyes are in each template (`views.<view>.eyes`: `x`, `y`, radius `r` in template pixels; multiply by the painting's scale). **Paint the eyes there**: a painted skin wears no eye dots on the stage, so the face is wherever the painting puts it. |
| `textures/<family>/outfits/<outfit>/…` | same, for every outfit a creature wears (`data/outfits/outfits.toml`) | The same set of templates, depth guides and marks for the family **dressed** in that outfit (robe, hood, cloak, …): the outline includes the garments. See `docs/clothing-kit-spec.md`. |
| `textures/jobs.json` | gs-studio: `gs-studio.exe --data <data root> --export-jobs` (headless; also *Export jobs* in the Rigs tab). It exports every template above first. | The job list: every bestiary creature a rig draws (627 today). |
| `textures/<…>/projected/{side,back}_{render,unpainted}.png` | gs-studio: `gs-studio.exe --data <data root> --project <family>[/<creature>]:side\|back` | Projection-first (`docs/projection-first-export-spec.md`): what the paintings so far show from the side / back, and the mask of what is left to paint. |

Template sizes today (width × height, px; back = front). They change when a
rig or silhouette changes, so always read the real file. Note that many side
views are **landscape** (four-legged animals, spiders, snakes):

| Family | Front / back | Side | | Family | Front / back | Side |
| --- | --- | --- | --- | --- | --- | --- |
| arachnid | 512 × 280 | 512 × 280 | | golem | 512 × 471 | 444 × 512 |
| avian | 289 × 512 | 512 × 280 | | humanoid | 186 × 512 | 88 × 512 |
| canine | 199 × 512 | 512 × 280 | | ogre | 359 × 512 | 331 × 512 |
| caprine | 268 × 512 | 512 × 474 | | orc | 352 × 512 | 222 × 512 |
| crustacean | 512 × 280 | 512 × 280 | | reptilian | 512 × 280 | 512 × 165 |
| elemental | 369 × 512 | 313 × 512 | | serpent | 512 × 280 | 512 × 280 |
| elephantid | 357 × 512 | 512 × 280 | | suine | 384 × 512 | 512 × 309 |
| equine | 170 × 512 | 512 × 415 | | treekin | 372 × 512 | 329 × 512 |
| feline | 363 × 512 | 512 × 280 | | troll | 419 × 512 | 396 × 512 |
| ghost | 391 × 512 | 264 × 512 | | ursine | 391 × 512 | 512 × 280 |
| giant | 385 × 512 | 127 × 512 | | worm | 375 × 512 | 512 × 280 |

`jobs.json` (version 1) — build against this shape:

```json
{
  "version": 1,
  "data_root": "E:/Gemstone/dev/gs_studio/data",
  "jobs": [
    {
      "creature": "forest troll",
      "key": "forest_troll",
      "family": "troll",
      "body_type": "biped",
      "height_ft": 9.0,
      "description": "Huge and dangerous, the forest troll towers above ...",
      "brief": "a huge hulking troll with thick cracked grey hide like stone plates, ...",
      "outfit": [],
      "outfit_key": null,
      "marks": "textures/troll/marks.json",
      "views": {
        "front": { "template": "textures/troll/front_paint.png", "depth": "textures/troll/front_depth.png", "size": [419, 512] },
        "back":  { "template": "textures/troll/back_paint.png",  "depth": "textures/troll/back_depth.png",  "size": [419, 512] },
        "side":  { "template": "textures/troll/side_paint.png",  "depth": "textures/troll/side_depth.png",  "size": [396, 512] }
      },
      "out_dir": "textures/troll/forest_troll"
    }
  ]
}
```

- `size` is the template's pixel size when the list was written. If the
  template on disk has another size, the rig changed since: skip the job and
  report it ("stale template") — do not generate against it. A depth guide
  must be exactly 4× its template; otherwise it is stale too.
- `depth` is present only for families whose puppet is approved (today:
  humanoid). No `depth` key → run without depth control.
- `brief` may be empty: then write one from `description` (section 8), or
  skip the job if `description` is empty too.
- A creature with an outfit (`"outfit": ["robe", "hood"]`,
  `"outfit_key": "robe+hood"`) has its `views` and `marks` in
  `textures/<family>/outfits/<outfit_key>/`: paint the garments into the
  dressed outline. Creatures with the same outfit share those templates.
- `out_dir` is the creature's own folder: `front.png`, `side.png`,
  `back.png` go there, and gs-studio builds that creature's skin from them
  (*Build every skin*). Several creatures drawn by one family each get their
  own skin.
- `brief` is always empty from gs-studio; `description` comes from the
  bestiary (empty for about 80 creatures).

### Outputs

For each job, in `<out_dir>/`:

| File | Rule |
| --- | --- |
| `front.png`, `back.png`, `side.png` | RGBA PNG. **Exactly the template's aspect ratio** (any size; write it at 4× the template, e.g. 744 × 2048 for a 186 × 512 template). Alpha = 255 inside the silhouette, 0 outside (take the alpha from the template, scaled up). |
| `meta.json` | What made it: model file, seed per view, prompt, workflow version, date, and review status (`pending` / `approved` / `rejected`). |

Hard rules, because of how gs-studio reads the paintings:

1. **Views.** Front: the creature faces the viewer (its right hand is on the
   image's left). Back: seen from directly behind, matching `back_paint.png`
   (already mirrored). Side: the creature seen from its **right** side,
   facing the image's **right** edge — match `side_paint.png`.
2. **No background colour anywhere.** Alpha 0 outside the silhouette means
   "not painted". gs-studio samples up to 6 px past a painted edge, so a
   coloured background bleeds into the skin. Cut every output to the
   template's alpha.
3. **Cover the whole silhouette.** A few pixels of overflow are harmless
   (they are cut); an unpainted hole shows as plain body colour.
4. **Paint only what fits inside.** No capes, wings, flames or hair past the
   outline — they are cut off anyway.
5. **Empty hands.** Weapons and shields are separate 3D props in gs-studio;
   painted ones end up smeared along the arm.
6. **Flat lighting.** The game shades the body itself. Painted shadows and
   highlights double up and look wrong when the creature turns.
7. **Never overwrite an approved painting** (check `meta.json`).

gs-studio side (not yours, listed so you know what exists when): a separate
skin per creature (today there is one per family), the `jobs.json` export, a
"build every skin" command, and a textured turnaround render for review.

## 3. Software stack

**Use ComfyUI on native Windows with AMD's ROCm 10.0 PyTorch wheels, run
headless and driven from Python over its HTTP API.** Research date
2026-09-24; re-check versions at install time.

What this rests on (all confirmed by a second, adversarial check):

| Fact | Source |
| --- | --- |
| AMD's ROCm 10.0.0 compatibility matrix (2026-08-19) names the **RX 7900 GRE (gfx1100)** for Windows 11 25H2 with Adrenalin 26.8.1+, and also lists WSL2 for it | rocm.docs.amd.com compatibility matrix |
| ComfyUI's README Windows-AMD route (PR #16217, 2026-09-10) uses torch 2.13.0+rocm10.0.0 on 64-bit Python 3.13, no separate HIP SDK; the wheels exist on AMD's index, including gfx1100-only packages | ComfyUI README |
| AMD TheRock marks gfx1100 on Windows "Release Ready" since 2026-07-03 | TheRock SUPPORTED_GPUS.md |
| RDNA3 has **no FP8 compute** (AMD: FP8 is RDNA4-only) | AMD Radeon docs |
| ZLUDA forks are superseded for RDNA3, DirectML is in maintenance mode, AMD Amuse is discontinued | fork README, Microsoft DirectML README, heise 2026-04-04 |
| Windows ROCm has **no training support** — no LoRA training on this PC under Windows | AMD Radeon limitations page |

Install (production = pinned git + venv; easiest to reproduce and script):

1. Windows 11 25H2, Adrenalin **26.8.1 or newer**, GPU at **stock clocks and
   voltage** (a reported "black image" case was the owner's undervolt).
   32 GB system RAM minimum, 64 GB preferred: large models are swapped
   between VRAM and RAM every job.
2. Install to a short ASCII path at a drive root (e.g. `D:\comfy`).
   64-bit Python 3.13, `git clone https://github.com/Comfy-Org/ComfyUI`,
   create a venv.
3. Install PyTorch from AMD's index, then ComfyUI's requirements, then lock:
   ```
   pip install --index-url https://stable.repo.amd.com/rocm/whl-next/ ^
     "torch[device-gfx1100]==2.13.0+rocm10.0.0" ^
     "torchvision[device-gfx1100]==0.28.0+rocm10.0.0" ^
     "torchaudio==2.11.0.2+rocm10.0.0"
   pip install -r requirements.txt
   pip freeze > lock.txt
   ```
4. List the GPUs before anything else:
   `python -c "import torch;[print(i, torch.cuda.get_device_name(i)) for i in range(torch.cuda.device_count())]"`.
   If an integrated GPU shows up, start ComfyUI with `--cuda-device <the
   7900 GRE's index>`. Do not blindly set `HIP_VISIBLE_DEVICES=0` — the dGPU
   is often index 0 and the iGPU index 1, and guessing wrong selects the iGPU.
5. Launch: `python main.py --listen 127.0.0.1 --port 8188 --disable-auto-launch --input-directory <texture-gen>/comfy_in --output-directory <texture-gen>/comfy_out`.
   Confirm the log says `Using pytorch attention` (ComfyUI enables it itself
   on gfx1100; `--use-pytorch-cross-attention` is harmless but redundant).
6. Leave `COMFYUI_ENABLE_MIOPEN` unset (ComfyUI disables MIOpen on RDNA3 for
   speed). Do not set `PYTORCH_TUNABLEOP_ENABLED` unless every job uses one
   fixed resolution.
7. If it crashes with `0xc0000005` / access violations while loading models,
   add `--disable-smart-memory`; as a last resort `--disable-dynamic-vram`
   (slated for removal upstream — not a long-term setting).

**Acceptance soak before any overnight run:** 200 images of the exact
production workflow, submitted through the API. Log `torch.__version__`,
`torch.version.hip` and the driver version. Pass only if nothing crashes and
seconds per image stay flat. This run is also the **throughput benchmark** —
no measured 7900 GRE numbers exist for any model below.

Fallbacks, in order:

- **A.** AMD's ROCm 7.14.x Windows wheels (documented on AMD's ai-ecosystem
  PyTorch page; keeps ComfyUI's Dynamic VRAM).
- **B.** ComfyUI's portable AMD build, `ComfyUI_windows_portable_amd.7z` from
  release v0.37.0 (ROCm 7.2, Python 3.12). Comfy-Org reverted it from 7.14
  back to 7.2 on 2026-07-28; its CI does not test it.
- **C.** WSL2, Ubuntu 24.04, ROCm 10.0 (or 7.2.1, whose WSL matrix also
  names the GRE). Keep models and outputs on the Linux filesystem.

Rules for this card on Windows:

- **One GPU process at a time.** Two ComfyUI processes on gfx1100 can drop one
  to ~2% speed until killed (TheRock #6824). No games or other GPU jobs
  during a batch, and unload the local LLM before generating (section 6).
- **Watch for slowdowns.** Runs slowing 3–4× after the first image have been
  reported (ComfyUI #16502, fixed by `POST /free`; ROCm #5834 on the 7900 XTX).
  The driver script tracks seconds per image and calls `/free`, or restarts
  ComfyUI, when it creeps up.
- **Fixed canvas sizes.** Every new shape re-runs kernel selection on Windows
  gfx1100 (TheRock #7958); a handful of fixed sizes keeps it fast.
- The Adrenalin "AI Bundle" and ComfyUI Desktop (v1.1.x, ROCm 7.14) are fine
  for a first look, not for the unattended run.

## 4. Models

**Start with SDXL; decide on anything bigger with a one-week bake-off on the
real card.** For this job — paint inside a mask, keep three views of one
creature consistent, fit 16 GB on AMD — the most mature and fastest stack is
SDXL with ControlNet and IP-Adapter. The newer edit models are better at
"the same creature from another angle" but are heavier and unproven on this
card.

Baseline (v1):

| Piece | Choice | Licence / note |
| --- | --- | --- |
| Checkpoint | One painterly SDXL fine-tune, e.g. DreamShaper XL. Must be a **regular, non-distilled** checkpoint — the Fooocus inpaint patch does not work with Turbo / Lightning / Hyper merges | CreativeML Open RAIL++-M |
| Control | xinsir ControlNet-Union SDXL ProMax (inpaint, lineart / canny, depth) | Apache 2.0 |
| Style lock | IP-Adapter Plus SDXL + its CLIP-vision model, weight type "style transfer", fed ONE fixed style image | Maintenance-only since 2025-04-14: pin its version and ComfyUI's |
| Masked fill | Acly `comfyui-inpaint-nodes` (Fooocus inpaint) or core "VAE Encode (for Inpainting)" | — |
| Format | fp16 `.safetensors` | — |

Bake-off candidates (run the section 5 recipe on the same ~10 creatures —
include a worm, an arachnid, a ghost and a humanoid — and time each):

| Model | Size that fits 16 GB | Licence | Why try it |
| --- | --- | --- | --- |
| Qwen-Image-Edit-2511 (20B) | Test both: unsloth GGUF Q4_K_M (13.2 GB, city96 ComfyUI-GGUF loader) and Comfy-Org `qwen_image_edit_2511_fp8mixed` (20.5 GB, partial RAM offload, wants 64 GB RAM). Text encoder `qwen_2.5_vl_7b_fp8_scaled` (~9 GB) is swapped in and out each job | Apache 2.0 | Best open "same subject from a new angle" edit; built-in new-viewpoint and relighting. Lightning 4-step LoRA for the batch. Test the base model before adding the Multiple-Angles LoRA |
| FireRed-Image-Edit-1.1 (20B) | Same loaders as 2511 | Apache 2.0 | Drop-in A/B against 2511; claims better identity consistency |
| HiDream-O1-Image (8B, 2026-05-08) | FP8 or bf16, native ComfyUI | MIT | Instruction editing + multi-reference, much lighter than 20B |
| FLUX.2 [klein] 4B | bf16 ~8 GB | Apache 2.0 | Fast multi-reference edit; weaker painterly style without a LoRA |
| Z-Image-Turbo (6B) + Fun-ControlNet-Union-2.1 (v2601/2602) | bf16 ~12 GB | Apache 2.0 | Fast (8 steps) with depth + inpaint control; `control_context_scale` 0.65–1.0, higher for inpaint |

Do not use:

- **Qwen-Image-2.1** (released 2026-09-20; masks, RGBA, 10 references —
  technically ideal): licence is research / evaluation only. Revisit if that
  changes.
- **FLUX.1 dev / Fill / Kontext / Redux, FLUX.2 dev, FLUX.2 klein 9B:**
  FLUX Non-Commercial licence (FLUX.2 dev is also ~19 GB even at Q4). Only
  if the owner explicitly accepts that licence.
- HiDream I1 / E1.1 (too heavy); SD 3.5 Large (weak inpainting); any
  `int8_convrot`, nvfp4 or Nunchaku model (slow or noisy on gfx1100, or
  NVIDIA-only).

Format facts for this card:

- No FP8 compute on RDNA3. ComfyUI keeps FP8 **storage** only for models
  over ~6.8B parameters on a 16 GB card; smaller FP8 files (klein 4B,
  Z-Image) are upcast to bf16 at load — they save disk, not VRAM.
  `--fp8_e4m3fn-unet` forces FP8 storage.
- GGUF speed on RDNA3 is disputed (one report of poor GPU use, one of Q8 ≈
  FP8). Benchmark it; do not assume.
- The GemStone IV creatures are Simutronics' IP; model licences do not
  cover that question and it was not assessed.

## 5. Recipe for one creature

**v1: all three views of one creature in one image, painted in one pass, so
they share one design; then refined per view and cut to the exact
silhouette in Python.** The silhouette is enforced in pixels by the cut, not
trusted to the model.

1. **Canvas (from gs-studio).** gs-studio will export, per family, a control
   sheet: FRONT | SIDE | BACK laid out at ONE shared pixels-per-foot scale
   (the individual `*_paint.png` templates each have their own scale — 512 px
   on the long side — so they cannot simply be pasted side by side), plus a
   mask and a `layout.json` with each view's box and its template. Until that
   export exists, run each view as its own job from its template (weaker
   consistency). Families differ wildly in shape (the three views together
   run from about 0.9:1 for humanoid to 6.8:1 for reptilian), so the sheet
   layout is chosen per family — one row or two — to stay near ~1 MP in an
   SDXL bucket with sides that are multiples of 64 (1024×1024, 1152×896,
   1216×832, 1344×768, 1536×640). Outside the silhouettes: flat **neutral
   mid-grey (#808080)**. Not magenta — a colour key would also delete real
   paint on crimson or violet creatures.
2. **Pass 1, whole sheet, one sampler call.**
   - Masked fill: mask = silhouettes grown 8–16 px, feather 0–4 px, inside
     pre-filled grey, denoise 1.0.
   - ControlNet-Union: lineart / canny of the silhouette outline at 0.4–0.6.
     **Depth, for approved families only (today: humanoid).** gs-studio's
     `*_depth.png` guides (section 2) show the body's form and where the
     joints sit, not just its edge. Use them at 0.7–0.9, ending at 0.85; in
     a control sheet, scale and place each guide exactly as its template (it
     has the template's framing at 4×), with black where the sheet is grey
     canvas. Feeding today's unapproved animal shapes would teach the model
     their defects, so those run without depth until approved.
   - IP-Adapter, weight type "style transfer", 0.6–0.8, the ONE fixed style
     image for all ~600 creatures.
   - Sampler: dpmpp_2m karras, 30 steps, CFG 5–6. 2–4 seeds per creature.
   - Seed: derived from `uuid5(NS, f"{key}/{attempt}").int & 0xFFFFFFFFFFFFFFFF`
     — never Python's `hash()`, which is salted per process and often
     negative (ComfyUI rejects negative seeds with HTTP 400).
   - Prompt: style block + `character turnaround sheet, orthographic, three
     views of the same {brief}: front view, right side view, back view, full
     body` (section 8). Add `face on back of head` to the negative.
3. **Pass 2, per view.** Crop each panel, upscale 1.5–2×, redo it with the same
   controls, style image and seed at denoise 0.35–0.5, with the view's own
   line (`back view, no face visible`).
4. **Post-processing, in Python.**
   - Crop the view's box, then set alpha = the template's alpha resized with
     NEAREST. This guarantees nothing outside the silhouette.
   - Pad colours 8–16 px outward under alpha 0 (edge padding), so filtering
     never pulls grey into the edge.
   - Untouched fill inside the body (flat canvas grey or flat template grey;
     painted grey has brush texture and is kept) → alpha 0 (gs-studio fills
     unpainted pixels from the nearest paint); more than ~2% → reject.
   - Colour-match side and back to the front (histogram or Reinhard).
   - Optional delight: divide by a heavily blurred luminance inside the mask
     and restore the mean. (Marigold V2 albedo and the Light_restoration
     LoRA exist but depend on `int8_convrot` / 2509 bases that misbehave on
     AMD — test before relying on them.)
   - Save straight (not premultiplied) RGBA PNG as `front.png`, `side.png`,
     `back.png` (section 2).

**Two-stage alternative (bake-off).** Paint FRONT with v1. Then paint SIDE and
BACK with an edit model (Qwen-Image-Edit-2511 / FireRed / HiDream-O1 /
klein 4B): image 1 = the approved front, image 2 = that view's silhouette,
prompt `the same creature from image 1 seen from its right side (from
behind), filling exactly the shape in image 2, flat unshaded colours`.
Then the same post-processing.

**Later (needs gs-studio work and approved puppets):** projection-first
consistency, as the StableGen Blender add-on does for meshes: paint FRONT,
project it onto the puppet (gs-studio's `prefill`), render the side from the
painted puppet, and inpaint only what the front could not see.

## 6. Batch automation

**One Python script drives ComfyUI's HTTP API; a manifest on disk is the
source of truth, because ComfyUI keeps its queue and history in memory
only.** Checked against ComfyUI master / v0.37.0 (2026-09-21). Dependencies:
`requests`, `Pillow`, `numpy`.

The workflow:

- Build it in the UI, then **File → Export Workflow (API)** →
  `workflows/paint_sheet.api.json`. Give the nodes the script edits unique
  titles and find them by `_meta.title` (fail on a missing or duplicate
  title): `IN_IMAGE` (LoadImage), `IN_MASK` (LoadImageMask, channel red,
  white = paint), `IN_POS`, `IN_NEG`, `IN_STYLE` (style image), `IN_SEED`,
  `OUT_SAVE` (SaveImage — writes to disk, so results survive a driver crash).
- At startup: `GET /system_stats`, `GET /object_info/<class>` for every node
  class used, and probe `GET /api/jobs` (builds before 2025-12-18 lack it:
  fall back to `/history` + `/queue`).

Per job (one creature; or one view when running per-view):

| Step | Call |
| --- | --- |
| Upload canvas and mask | `POST /upload/image`, multipart `image=(name, bytes, "image/png")`, `type=input`, `subfolder=texgen`, `overwrite=true`; set LoadImage's `image` to `"{subfolder}/{name}"` from the response. Upload masks the same way (not `/upload/mask`, which copies alpha into another image) |
| Submit | `POST /prompt` `{"prompt": wf, "prompt_id": str(uuid5(NS, f"{key}#{attempt}")), "client_id": "texgen"}`. Set `filename_prefix` to `texgen/{key}/{view}_a{attempt:02}`. A 400 returns `node_errors`: mark the job failed and continue — but re-queue it after a code fix, not permanently |
| Wait | Poll `GET /api/jobs/{prompt_id}` every 2 s until `completed` / `failed` / `cancelled`; a 404 means ComfyUI restarted — resubmit with the same id |
| Fetch | `GET /history/{prompt_id}` → `outputs[<OUT_SAVE id>].images[]` → `GET /view?filename=&subfolder=&type=`; save every attempt under `jobs/<key>/raw/` |

Queue, resume and watchdog:

- Keep at most 1–2 prompts inside ComfyUI. Job states in SQLite (or per-job
  `job.json` written atomically with `os.replace`): `pending → prompted →
  submitted(prompt_id) → generated → cut → auto_rejected | awaiting_review →
  approved | redo → promoted`. On start, skip jobs whose final output
  exists; ask `/api/jobs/{id}` about every `submitted` one.
- Watchdog: no progress and no `/system_stats` answer for N minutes, or a job
  past its timeout → `POST /interrupt {"prompt_id": ...}`, kill and restart
  the ComfyUI process, resubmit. Give up after 3 attempts per job.
- Speed guard: if seconds per image rise ~50% above the run's median, call
  `POST /free {"unload_models": true, "free_memory": true}`; if that does not
  help, restart ComfyUI. Also restart every N images as a precaution.
- Overnight: call `SetThreadExecutionState(0x80000001)` (ES_CONTINUOUS |
  ES_SYSTEM_REQUIRED) so Windows does not sleep; set Windows Update active
  hours; start from Task Scheduler with a `--until 07:00` stop time; order
  the work by family.
- Budget: ~600 creatures × 3 views = 1,800 view images. One 8–10 hour night
  needs ≤ 16–20 s per image including retries — unproven on this card; plan
  for several nights and measure on night 1.

A tested starting point for prep and cut (checked on the real humanoid
templates: zero painted pixels outside the silhouette). Adapt `prep` to the
control sheet once gs-studio exports it:

```python
import uuid, numpy as np
from PIL import Image, ImageFilter

GREY = (128, 128, 128)                      # canvas fill outside the body
FILL = (205, 205, 205)                      # the template's body grey
NS = uuid.UUID("6f1c7c3e-2b7a-4c1e-9a55-3f0d1c2b9e10")

def seed_for(key, attempt):                 # never Python's hash()
    return uuid.uuid5(NS, f"{key}/{attempt}").int & 0xFFFFFFFFFFFFFFFF

def prep(template_png, canvas_wh=(832, 1216), scale=2):
    t = Image.open(template_png).convert("RGBA")
    t = t.resize((t.width * scale, t.height * scale), Image.NEAREST)
    cw, ch = canvas_wh
    x, y = (cw - t.width) // 2, (ch - t.height) // 2
    canvas = Image.new("RGB", (cw, ch), GREY)
    canvas.paste(t.convert("RGB"), (x, y), t)
    mask = Image.new("L", (cw, ch), 0)
    mask.paste(t.getchannel("A"), (x, y))  # white = paint here
    return canvas, mask, (x, y, t.width, t.height)

def cut(raw_png, template_png, box):
    x, y, w, h = box
    raw = Image.open(raw_png).convert("RGB").crop((x, y, x + w, y + h))
    alpha = Image.open(template_png).convert("RGBA").getchannel("A").resize((w, h), Image.NEAREST)
    rgb = np.asarray(raw).astype(np.int16)
    a = np.asarray(alpha).copy()
    inside = a > 0
    near = lambda c: np.abs(rgb - np.array(c)).max(axis=-1) < 6
    # Untouched fill is grey AND perfectly flat; painted grey (a troll's
    # hide) has brush texture, so it is kept.
    flat = np.asarray(raw.convert("L").filter(ImageFilter.FIND_EDGES)) < 3
    residue = (near(GREY) | near(FILL)) & flat & inside
    frac = residue.sum() / max(inside.sum(), 1)
    a[residue] = 0                          # gs-studio fills from nearest paint
    out = Image.fromarray(np.dstack([rgb.astype(np.uint8), a]), "RGBA")
    return out, {"residue_frac": float(frac), "ok": frac <= 0.02}
```

**Local LLM for briefs (phase 0, a separate run before any images).** Only if
briefs are not supplied in `jobs.json`. A 7–14B instruct model at Q4 is
enough: `qwen3.5:9b` (6.6 GB, also takes images — useful for QA later),
`qwen3:8b` or `gemma4:12b`.

- Ollama: `POST http://localhost:11434/api/chat` with `{"model": "qwen3.5:9b",
  "stream": false, "think": false, "format": <JSON schema: subject, palette[≤5],
  front, back, side>, "options": {"temperature": 0.3, "seed": 7, "num_ctx":
  8192}, "messages": [system rules, description]}`; parse
  `json.loads(r["message"]["content"])`. Use the native `/api/chat`
  `format` — Ollama's OpenAI-compatible endpoint documents only JSON mode.
- LM Studio: `POST http://localhost:1234/v1/chat/completions` with
  `response_format: {"type": "json_schema", "json_schema": {"name":
  "creature_visual", "strict": true, "schema": ...}}`; parse
  `choices[0].message.content`.
- System rules: describe only what is visible on the body; no lore,
  behaviour, sound, smell, or anything held; 25–40 words.
- Cache results in `jobs/<key>/prompt.json`. Python builds the final prompt
  (style block + brief + view line). Keep it under ~60 words for SDXL's CLIP.
- **Unload the LLM before generating images**: Ollama `keep_alive: 0` or
  `ollama stop <model>`; LM Studio `lms unload --all`.

**Review.** Each morning the script writes `review/index.html`: one row per
creature, the three cuts on a checkerboard, keyboard approve / reject / redo,
exported as `review/decisions.json`, which the next night's run reads.
Approved cuts are then copied ("promoted") into `textures/<family>/<key>/`
(section 2). The best review shows the result on the puppet; gs-studio will
add a textured turnaround render for that.

## 7. Folder layout for the generator

```
texture-gen/                     (its own folder or repo, NOT inside gs_studio/crates)
  workflows/
    paint_sheet.api.json         ComfyUI workflow, exported in API format
  styles/
    style.txt                    the fixed style block (section 8)
    negative.txt                 the fixed negative prompt
    style_ref.png                the ONE fixed style image (IP-Adapter)
  generate.py                    reads jobs.json, drives ComfyUI, writes outputs
  review.py                      builds the review page, records approvals
  state.sqlite                   job states and seeds (section 6)
  jobs/<key>/                    job.json, prompt.json, input/, raw/ (every attempt), cut/, qa.json
  review/                        index.html, decisions.json
  comfy_in/, comfy_out/          ComfyUI's --input-directory / --output-directory
  logs/
```

## 8. Prompts

Every prompt = the fixed **style block** + one **view line** + the
creature's **brief**. Only the brief changes between creatures; that is what
keeps the set uniform. Keep the style block and negative prompt in files and
never edit them per creature.

Style block:

```
hand-painted dark-fantasy creature concept art, matte, soft visible brushwork,
realistic anatomy, mid saturation, full body, single character, flat even
frontal lighting, no cast shadows, no rim light, no dramatic highlights,
no ambient occlusion, empty hands
```

Negative prompt (for models that take one):

```
shadow, cast shadow, dramatic lighting, background, scenery, floor, text,
letters, watermark, signature, frame, border, grid, outline, multiple
characters, extra limbs, cropped, weapon, shield
```

View lines: `seen straight from the front` / `seen straight from behind` /
`seen from its right side, facing right`.

**Brief**: 25–40 words describing only what the creature looks like — body
shape, skin / fur / scales / armour with colours and materials, face,
distinctive features. Drop lore, behaviour, smell, sound, and anything held.
Example, from the forest troll's bestiary description ("Grey skin so thick
that it serves quite well as armor covers most of the troll, with tufts of
thick hair sprouting here and there like weeds between cracked stones. A
hideous grin splits its face displaying fangs crusted with dried blood..."):

> a huge hulking troll with thick cracked grey hide like stone plates, tufts
> of coarse dark hair sprouting between the cracks, a wide grin of
> blood-crusted fangs, small narrow piggish eyes

Write the briefs once, review them, and store them in `jobs.json`, so a
re-run never changes a creature's look. If they must be generated, use a
local LLM (section 6). Natural sentences suit Flux-class models; an SDXL
model may do better with the same words as comma-separated tags — pick one
form for the whole set.

## 9. Quality checks

The script rejects mechanical failures itself and re-queues them with a new
seed; a person reviews only what passes.

| Check | Automatic test (after cutting to the template alpha) | On failure |
| --- | --- | --- |
| Coverage | ≥ 98% of silhouette pixels painted (not transparent, not the canvas grey) | Re-run, new seed |
| Background leak | No canvas-grey (#808080), near-white or near-black band in the 4 px just inside the outline | Re-run |
| Baked lighting | Left and right halves of the figure within 15% mean brightness; no large region under 20% brightness | Flag for review |
| Views disagree | Front, back, side hue/value histograms within tolerance of each other | Re-run back and side from the approved front |
| Style drift | Saturation and brightness within the batch's normal range | Flag for review |

Reviewer rejects: text, signatures, watermarks, frames or grids; anything
held; a second figure or extra limbs; a face anywhere but the head; strong
shadows or a spotlight look. Loose brushwork and small overflow are fine.

Final check is in gs-studio: build the skin and look at the creature turning.

## 10. Build order

Generator milestones (yours), each with its check:

| # | Milestone | Done when |
| --- | --- | --- |
| M0 | Install, GPU check, soak | 200 API-submitted images, no crash, flat seconds per image; throughput recorded |
| M1 | v1 look, by hand in the ComfyUI UI, on the humanoid family's templates | A style image and settings the owner approves |
| M2 | Driver script, one creature end to end | Its three PNGs meet every rule in section 2 and build into a skin in gs-studio |
| M3 | Bake-off (section 4) on ~10 varied creatures | A chosen model and recipe, with timings |
| M4 | Batch, resume, watchdog, review page | A 50-creature overnight run that survives a forced ComfyUI kill mid-run |
| M5 | Pilot | 10 humanoid-family creatures approved and built into skins |
| M6 | Full run, family by family | Only families whose gs-studio puppet is approved |

gs-studio work (ours), in the order the milestones need it:

| # | Deliverable | Needed by |
| --- | --- | --- |
| G1 | A separate skin per creature, and a "build every skin" command | **Studio side done**: creature folders (`out_dir`), *Build every skin*. The stage does not show per-creature skins yet |
| G2 | `jobs.json` export (section 2) | **Done**: `--export-jobs` |
| G3 | Control sheet export per family: views at one shared scale, mask, `layout.json` | M3 (per-view jobs until then) |
| G4 | Textured turnaround render for review | M5 |
| G5 | Depth maps and the projection pass, from approved puppets only | **Done**: `*_depth.png` from Export templates; the projection pass (depth-tested) was already in `prefill`. Humanoid approved (`data/generator.toml`) |
| G6 | Projection-first renders (`docs/projection-first-export-spec.md`) | **Done**: `--project <family>[/<creature>]:side\|back` |

## 11. Risks and open questions

Risks:

- **Windows ROCm 10 is five weeks old.** Open issues bite unattended runs:
  slowdowns after the first image (ROCm #5834, ComfyUI #16502), a Flux-dev
  load crash on 7.14 (ROCm #6438), two GPU processes starving each other
  (TheRock #6824), kernel search never cached (TheRock #7958). Mitigations:
  the soak, pinned versions, the watchdog and speed guard, fallbacks A–C.
- **Throughput is unknown.** No measured 7900 GRE numbers exist for any of
  these models. The nearest are the faster 7900 XTX's (SDXL 1024² ~7 s; a
  12B edit model ~17 s at 8 steps) and the GRE has roughly 75% of its compute
  and 60% of its memory bandwidth. Assume several nights.
- **Consistency across views is unproven.** That is why v1 paints all three
  views in one image and why the bake-off exists. Colour histograms only
  catch gross mismatches; the reviewer is the real check.
- **Odd shapes.** Worms, arachnids, serpents and ghosts may not fill their
  silhouettes, or may get faces on the back of the head.
- **The animal puppets are not approved yet.** Their 3D shapes still have
  known defects (being reworked in gs-studio). Do not use their depth maps
  (gs-studio writes them for every family; only humanoid's are approved),
  and expect paintings wrapped onto them to show the defects. Start with the
  humanoid family.
- **Licences.** Qwen-Image-2.1 is research-only; the FLUX dev family is
  non-commercial; the GemStone creatures are Simutronics' IP. None of this
  is legal advice.
- **No LoRA training on Windows ROCm.** If a style LoRA is wanted (after
  ~30–50 approved creatures), train it under WSL2/Linux or in the cloud.

Open questions for the owner:

- How much system RAM the PC has (32 GB minimum, 64 GB for the fp8 20B route).
- Whether the FLUX non-commercial licence is acceptable for a free fan tool.
- Which families' puppets are approved at the time of each run.

## 12. Sources

Checked 2026-09-24; each was opened, and the key claims were re-checked by a
second, adversarial pass.

- AMD ROCm 10.0.0 compatibility matrix (2026-08-19) — https://rocm.docs.amd.com/en/latest/compatibility/compatibility-matrix.html
- ComfyUI README, Windows AMD install (PR #16217, 2026-09-10) — https://github.com/Comfy-Org/ComfyUI/blob/master/README.md
- ComfyUI portable AMD revert to ROCm 7.2 (PR #15127, 2026-07-28) — https://github.com/Comfy-Org/ComfyUI/pull/15127
- AMD TheRock supported GPUs (gfx1100 Windows Release Ready, 2026-07-03) — https://github.com/ROCm/TheRock/blob/main/SUPPORTED_GPUS.md
- AMD Radeon Windows compatibility and limitations (FP8 RDNA4-only; no training) — https://rocm.docs.amd.com/projects/radeon-ryzen/en/latest/docs/limitations/limitationsrad.html
- AMD ai-ecosystem PyTorch install page (7.14.x Windows wheels) — https://rocm.docs.amd.com/projects/ai-ecosystem/en/latest/frameworks/pytorch/install.html
- Windows gfx1100 issues — https://github.com/ROCm/ROCm/issues/5834 · https://github.com/ROCm/ROCm/issues/6438 · https://github.com/ROCm/TheRock/issues/6824 · https://github.com/ROCm/TheRock/issues/7958 · https://github.com/Comfy-Org/ComfyUI/issues/16502
- ComfyUI dtype rules (FP8 storage threshold) — https://github.com/Comfy-Org/ComfyUI/blob/master/comfy/model_management.py
- Qwen-Image-Edit-2511 — https://github.com/QwenLM/Qwen-Image · https://huggingface.co/unsloth/Qwen-Image-Edit-2511-GGUF · https://docs.comfy.org/tutorials/image/qwen/qwen-image-edit-2511
- Multiple-Angles LoRA — https://huggingface.co/fal/Qwen-Image-Edit-2511-Multiple-Angles-LoRA
- HiDream-O1-Image — https://huggingface.co/HiDream-ai/HiDream-O1-Image
- FLUX.2 (klein licences) — https://github.com/black-forest-labs/flux2 · https://docs.comfy.org/tutorials/flux/flux-2-klein
- Qwen-Image-2.1 licence (research only) — https://huggingface.co/Qwen/Qwen-Image-2.1/blob/main/LICENSE
- Z-Image-Turbo Fun ControlNet Union 2.1 — https://huggingface.co/alibaba-pai/Z-Image-Turbo-Fun-Controlnet-Union-2.1
- xinsir ControlNet-Union SDXL — https://huggingface.co/xinsir/controlnet-union-sdxl-1.0
- Acly inpaint nodes — https://github.com/Acly/comfyui-inpaint-nodes
- IP-Adapter Plus (maintenance notice) — https://github.com/cubiq/ComfyUI_IPAdapter_plus
- StableGen (multi-view mesh texturing reference design) — https://github.com/sakalond/StableGen
- ComfyUI server routes and API workflow format — https://docs.comfy.org/development/comfyui-server/comms_routes · https://docs.comfy.org/development/api-development/workflow-api-format · https://github.com/Comfy-Org/ComfyUI/blob/master/server.py
- ComfyUI jobs API (PR #14493) — https://github.com/Comfy-Org/ComfyUI/pull/14493
- Ollama chat API — https://docs.ollama.com/api/chat · model tags https://ollama.com/library/qwen3.5/tags
- LM Studio structured output — https://lmstudio.ai/docs/developer/openai-compat/structured-output
- SetThreadExecutionState — https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate
- Transparent-pixel edge bleed — https://www.adriancourreges.com/blog/2017/05/09/beware-of-transparent-pixels/
