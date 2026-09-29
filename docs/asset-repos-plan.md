# gs_studio and gs_studio_assets on GitHub — plan

Status: plan, 2026-09-28. Nothing here is built yet. Audience: the project
owner, who makes the decisions in section 9, and whoever builds it (a person
or an LLM agent). Where this plan is unsure of a fact, it says so and names
the phase that checks it.

## 1. The shape

Two public repositories:

| Repo | Holds | Publishes |
| --- | --- | --- |
| `gs_studio` (this one) | Source code: the app, the shared crates (`gs_field`, `gs_field_egui`, `gs_puppet`, ...), rigs, poses, the built-in puppets | App releases: `gs-studio.exe`, a Linux build of the same binary for the asset pipeline, checksums |
| `gs_studio_assets` (new) | Art, sorted into packs: backdrops and scenes, scenery props, creature skins, creature art | Asset packs as release zips, plus one `catalogue.json` listing every pack |

The flow for art:

```
submitter --> issue form (gs_studio_assets)
          --> Action: validate + process   (depth, sky, skins, sidecars, previews)
          --> Action opens a PR            (branch submission/issue-<n>)
          --> a collaborator reviews it --> merge to main
          --> Action builds the changed packs --> GitHub release + catalogue.json
          --> gs-studio and Hydra list, download, verify and install packs
```

The flow for code:

```
push / PR to gs_studio --> CI (tests)
tag v<x.y.z>           --> release workflow --> GitHub release with the binaries
```

The two meet in two places:
- The asset repo's Actions run tools from a pinned `gs_studio` release (`tools/pins.toml` in the asset repo).
- Both clients install packs with the same code, `gs_field::packs`.

## 2. Start from vellum-assets

`Nisugi/vellum-assets` (public) already runs the first half of this for
VellumFE's UI art, and it works. Its `SUBMISSIONS.md` describes it:

- One issue form per category (`.github/ISSUE_TEMPLATE/submit-*.yml`). Each form applies the labels `submission` and `category:<x>`.
- `.github/workflows/submission.yml` runs on issues opened, edited or labelled. It calls `.github/scripts/process_submission.py`, which:
  - parses the form, and turns the name into a strict path-safe slug;
  - downloads the attachment, accepting only GitHub-hosted URLs, with a 25 MB cap and a limit on pixels decoded;
  - rejects a name that's already taken;
  - removes black backgrounds where the category calls for it;
  - writes the PNG and a `.toml` sidecar, then opens a PR from `submission/issue-<n>`.
- A rejection is posted as a comment on the issue. When the submitter edits the issue, it runs again and force-updates the same PR.
- `manifest.yml` runs on every push to main. It builds Jinx-compatible `manifest.json` files (`build_manifest.rb`) and deploys the repo to GitHub Pages.

`gs_studio_assets` copies the submission design and the safety rules from
`process_submission.py`. It changes five things:

1. **Heavier processing.** It computes depth maps with a model, sky masks, and creature skins built from paintings. Those need the `gs_studio` binary and Python with torch, not just Pillow.
2. **Two jobs instead of one.** The job that touches the upload has no write access. A second job, with write access, only commits files the first job produced (section 5.3). vellum-assets gives the one job that handles untrusted input `contents: write`.
3. **A bot app opens the PRs.** A PR opened with the workflow's own `GITHUB_TOKEN` doesn't start other workflows, so PR checks would never run on submissions. A GitHub App token does start them (section 5.3).
4. **Packs on Releases instead of files on Pages.** A Pages site is capped at 1 GB. Scene art will pass that, while a release file can be up to 2 GB and there's no limit on releases.
5. **It stores the source paintings as well as the finished skins,** so skins can be rebuilt automatically after a rig change (section 5.6).

vellum-assets' UI categories (dolls, icons, frames, compass, hands, UI
skins) can stay where they are. See decision 8.

## 3. gs_studio (the source repo)

### 3.1 Going public — checklist

Checked on 2026-09-28:

- [x] **Dependencies:** the egui fork (`Nisugi/egui`, pinned to rev `ed8b2649`, as Hydra) is public, so a public checkout builds without credentials. There are no other git dependencies.
- [x] **Secrets:** none found in tracked files. `.cargo/config.toml`, which holds the local pool path, is untracked and stays that way.
- [x] **Size:** the repo takes 162 MB on GitHub, and 201 commits are all from one author address (`nisugi-gs4@gmail.com`), which becomes public.
- [ ] **Licence:** every crate says `license = "GPL-3.0-or-later"`, but there's no `LICENSE` file. Add the GPL text.
- [ ] **Local paths:** six tracked files contain paths like `G:\dev\...` (CLAUDE.md and docs). They're harmless, but scrub or generalise them if you'd rather.
- [ ] **README and CLAUDE.md:** they still say to keep VellumFE compatibility. VellumFE is retiring for Hydra, so update them.
- [ ] **Game content:** decide what the repo says about it (decision 2). `crates/gs_field/data` holds the bestiary and creature names from GemStone IV.

### 3.2 Workflows

**`ci.yml`** runs on pushes and on PRs to main.
- `cargo test --workspace` on `windows-latest`, cached with `Swatinem/rust-cache`.
- The rig tools' Python tests: `python -m unittest tools/test_refit_joints.py tools/test_snap_joints.py`.
- Check in phase 1: a test that needs a GPU adapter or a local pool must skip itself cleanly on a runner.

**`release.yml`** runs on a pushed tag `v*`, or by hand.
- Build `gs-studio.exe` on `windows-latest` (`cargo build --release -p studio`).
- Build the same binary on `ubuntu-latest` for the asset repo's Actions. They use its headless commands (section 3.3c).
  - Check in phase 1: the studio hasn't been built on Linux yet. Hydra's CI builds the same egui fork on Linux, so a working recipe exists.
- Write `SHA256SUMS` and create the GitHub release. Its notes are generated from the commits since the last tag.
- The exe is unsigned, so Windows SmartScreen warns on first run. Signing costs money; leave it for later (decision 10).
- The `bundled-art` build becomes optional once packs exist:
  - The plain exe offers to download packs on first run.
  - An "offline" exe can still be made by downloading the current packs and pointing `GS_STUDIO_EMBED_POOL` at them.
  - `.cargo/config.toml` stays per-machine.

Tags and releases are pushes too: one is made only when the owner asks for
that release.

### 3.3 Code to write in gs_studio

**a. `gs_field::packs`** installs packs, for both gs-studio and Hydra.
- It fits the crate's rules: Android-safe, no drawing, no HTTP. It needs a zip reader and a SHA-256 hash, both pure Rust.
- It reads the catalogue and the manifest, checks a download's SHA-256, and installs, updates or removes packs in the pool.
- It keeps a ledger at `<pool>/packs/installed.toml`, recording each pack's version and the hash of every file it installed.
- An update replaces only files still identical to what the pack installed, and keeps files the user changed and says so. That fixes what today's `bundled-art` seeding can't: it adds missing files but never updates an old copy.

**b. A Packs panel in gs-studio** (under Settings, or its own tab).
- It fetches `catalogue.json` from a configurable URL, defaulting to the asset repo.
- It lists packs as installed, update available or not installed, and does install, update and remove.
- HTTP lives only in the studio crate. It runs on a worker thread, with a blocking client using rustls.

**c. Headless commands** for the asset repo's Actions, added to the existing binary. It already has `--build-sheet`, `--save` and `--project`.
- `--validate <kind> <folder>`: checks the file types, sizes, alpha or opaque as the kind requires, names, sidecars and scene TOML, and that bestiary names and forms exist.
- `--sky <backdrop>`: the sky mask and optional clear sky. This is a Rust port of `image-gen/scenery/sky_clean.py`, which isn't in any repo today, so it runs anywhere without Python.
- `--pack <folder>`: a reproducible zip plus the manifest.
- `--catalogue <packs folder>`: writes `catalogue.json`.
- If building the GUI on Linux causes trouble, move these into a small separate binary instead. `editor/` and `stage/` are already free of the GUI library (the tests enforce it), so the split is mechanical.

**d. Depth maps:** they stay on `tools/depth_maps.py` (Depth Anything V2) for now, run on CPU in CI.
- Phase 3 measures how long a backdrop and a creature take on a runner.
- Running the model inside the app (ONNX) is the later step, if players or Niffy ever need to process art themselves.

**e. A Backdrops view in the Scenery tab**, next to today's prop calibrator (*Props*), like the Dolls tab's two views.
- It shows every backdrop in `global/images/scenes` with its sky overlay and what it's missing.
- It has toggles for *No sky* and *Clear the sky*, and one button to process everything that's missing something.
- It uses the same code as the Action, so a backdrop processed locally matches one processed on GitHub.

**f. Export for submission.** A button in the Creatures, Scenery, Scenes and Rigs (texture) tabs.
- It writes a zip laid out the way the matching issue form expects, then opens that form in the browser with the text fields filled in from the URL.
- GitHub forms can't be given a file through the URL, so the user drags the zip in.

**g. Pack format version.** The manifest carries a `format` number.
- A change to what the stage expects bumps it: a new depth encoding, or per-facing backdrops in the scene format.
- A client ignores packs newer than it understands, and says so.

## 4. gs_studio_assets — layout

```
gs_studio_assets/
  README.md               what is here, how to install packs, how to submit
  CONTRIBUTING.md         the art rules per kind (section 4.2) and the review checklist
  LICENSE                 the art licence (decision 2)
  .github/
    CODEOWNERS            who reviews what (decision 9)
    ISSUE_TEMPLATE/
      config.yml          blank issues off; links to CONTRIBUTING
      submit-backdrop.yml
      submit-prop.yml
      submit-skin.yml
      submit-creature.yml
    workflows/
      submission.yml      issue -> validate -> process -> PR   (5.1-5.3)
      pr-check.yml        every PR: validate the packs it touches  (5.4)
      release-packs.yml   push to main: build changed packs, release, catalogue  (6)
      rebuild.yml         new gs_studio release: rebuild derived files, open a PR  (5.6)
  tools/
    process_submission.py from vellum-assets, extended per kind
    pins.toml             which gs_studio release and depth model the Actions use
  packs/
    <pack-id>/
      pack.toml           id, title, kind, rooms, credits, licence
      global/...          the files, laid out exactly where they install in the pool
      sources/...         what derived files are rebuilt from (paintings); never installed
```

A pack is a folder in the pool's own layout, so installing one means copying
its `global/` tree into the pool. Example packs, from today's pool:

| Pack | Kind | Contents today | Size |
| --- | --- | --- | --- |
| `ojandhaart` | area | 5 backdrops and 4 Great Hall views (each with depth and sky), 21 scenes, 5 props | ~19 MB |
| `pilot-backdrops` | area | 7 backdrops: desert, snow, shop, sewers, forests (with depth and sky) | ~24 MB |
| `humanoid-skins` | skins | 10 creature skins and the paintings behind them | ~13 MB + sources |
| `canine-skins`, `troll-skins` | skins | 5 skins each | ~7 MB each + sources |
| `creatures-<region>` | creatures | sprite art with depth, normal maps and sidecars (optional; not in the first phases) | — |

**Storage:** plain git, with no LFS at the start.
- Files are 0.5–2 MB each. GitHub's advice is to keep a repo under about 1 GB, and it rejects single files over 100 MB.
- Both the source and the processed files are committed, so a reviewer sees exactly what ships. Git LFS only makes sense later: its free quota is small, and every CI checkout uses up its bandwidth.
- The injury dolls (719 MB, about 8 MB each) need shrinking before they join any repo.

### 4.2 Asset kinds

Each kind has its own issue form. Attachments go into a textarea: PNGs up to 10 MB each, or a zip up to 25 MB (GitHub's limits for issue attachments).

| Kind | Submitter attaches | Form fields | Validation | Processing | What lands in the pack |
| --- | --- | --- | --- | --- | --- |
| **Backdrop / scene** | The backdrop (PNG/JPG, opaque, landscape); optionally its own depth or sky | name, pack, room uid(s) or title it binds to, indoor/outdoor, sky: *clear it* / *keep the painted sky* / *no sky*, camera preset | opaque; size and aspect near the stage's; name free; room binding well-formed | depth map; sky mask; clear sky if asked; scene TOML written (camera, light) | `global/images/scenes/<name>.{png,depth.png,sky.png}`, `global/scenes/<uid> - <title>.toml` |
| **Scenery prop** | PNG with alpha, or on a green cove | name, pack, size (ft or m), feet anchor | alpha present (or keyable), size given | key the green if needed; write the sidecar | `global/images/scenery/<name>.{png,toml}` |
| **Creature skin** | `front.png`, `side.png`, `back.png` painted on the projection renders | form, creature | form exists; creature is in the bestiary and drawn by that form; paintings match the render sizes of the pinned rig | build the sheet and normal map with `--build-sheet`; check it isn't stale against the rig | `global/puppets/skins/<form>/<creature>.png` (+ normal); paintings in `sources/` |
| **Creature art** (later) | the sprite PNG with alpha (+ `_prone`) | creature, pack; calibration from gs-studio's export | creature in the bestiary; alpha; sidecar sane | depth and normal maps (`depth_maps.py`) | `global/images/creatures/<name>/...` |

Rules that belong in `CONTRIBUTING.md`, all learned the hard way:
- Backdrops have a clear, cloudless sky, because the stage draws its own moons, stars and clouds.
- No people in backdrops.
- Nothing painted lit: the stage lights bonfires and lamps itself.
- Giant places use giant-sized props, and doorways about 1.3 times a gigas.
- Skins: no nudity, and no faces painted on backs.

## 5. Submissions

### 5.1 The forms

Each form applies `submission` and `kind:<x>`, like vellum-assets. Every form has:
- a name and author credit;
- a required checkbox: *I made this or have the right to share it, it may be AI-generated (say which model), and I agree to the repo's licence.*

### 5.2 Validate and process: job `process`, no write access

- **Trigger:** issues opened, edited or labelled with `submission`. One run per issue at a time, and an edit cancels the run in progress (vellum-assets' `concurrency` block).
- **Permissions:** `contents: read`, `issues: write` so it can comment on the issue, and nothing else.
- **Input:** the issue body arrives through an environment variable. It is never written into a shell command, since `${{ github.event.issue.body }}` inside `run:` is a script-injection hole. Downloads are taken only from `github.com/user-attachments` and `*.githubusercontent.com`.
- **Caps:** 25 MB per attachment; a pixel cap set before any decode; zip entries checked for `..` paths and total unpacked size; no symlinks.
- **Tools:** it downloads the `gs_studio` Linux binary named in `tools/pins.toml` and checks its SHA-256. It checks out `gs_studio` at the same tag for `data/` (rigs, outlines, poses), which skins need.
- **Depth model:** Python, torch for CPU and the model weights come from `actions/cache` (the cache holds 10 GB per repo; Depth Anything Large is about 1.3 GB).
- **Output:**
  - the processed files, written under `packs/<pack>/`;
  - preview images: for a backdrop, the image, its depth and a sky overlay; for a skin, a turnaround render from the `turnaround` example, which runs on the CPU;
  - `summary.md`.
  
  All of it is uploaded as a workflow artifact.
- **A rejection** becomes a comment giving the reason and the fix. The job exits there.

### 5.3 Propose: job `propose`, write access through the bot app

- **The app:** a GitHub App (for example `gs-assets-bot`) installed only on `gs_studio_assets`, with contents and pull-request write.
  - `actions/create-github-app-token` turns its key, stored as a repo secret, into a short-lived token.
  - PRs the app opens start `pr-check.yml`; PRs opened with `GITHUB_TOKEN` would not.
- **What it does:**
  - Downloads the artifact and checks every path is under `packs/<pack>/`, with an allowed extension (`.png .jpg .toml`) and no overwrites outside the submission's own names.
  - Commits to `submission/issue-<n>` and opens or updates the PR ("Closes #n").
  - Pushes the preview images to an orphan `previews` branch, so they never reach main. It links them in the PR and the issue, where they show inline.
- **Reviewing images:** GitHub shows changed images in the PR's file view, with side-by-side, swipe and onion-skin comparisons. A reviewer sees the real files as well as the previews.

### 5.4 PR checks: `pr-check.yml`

This runs on every PR, from the bot or a collaborator's own branch.
- `--validate` on every pack the PR touches.
- The pack's size against its budget (decision 4).
- No files outside `packs/`, except for maintainers' own changes to `.github/` and `tools/`.
- `pack.toml` parses, and its rooms match the scenes' bindings.

### 5.5 Review and merge

- A ruleset on `main` requires:
  - a PR;
  - one approval from `CODEOWNERS`;
  - `pr-check` passing.
- Nobody pushes straight to main, and force-pushes to main are blocked.
- The reviewer's checklist is the rules in section 4.2, plus:
  - the scale looks right next to a gigas;
  - the name and room are right;
  - the licence box is ticked.
- Merging closes the issue.

### 5.6 Rebuilds after a gs_studio release

A rig change leaves every skin of that form stale: its sheet was cut for the
old layout. That happened to the humanoid's ten skins on 2026-09-28.
`rebuild.yml` automates the fix.

- **Trigger:** a new gs_studio release. `release.yml` sends a `repository_dispatch` using the bot app's token, backed by a daily scheduled check of the latest release.
- **What it does:**
  - Bumps `tools/pins.toml`.
  - Re-runs `--build-sheet` from `sources/` for every skin whose form changed.
  - Re-runs depth and sky if the tools' version changed.
  - Opens one PR with the results.
- A collaborator reviews that PR like any other.

## 6. Releasing packs: `release-packs.yml`

- **Trigger:** a push to main that changes `packs/`, or run by hand.
- **For each changed pack:**
  - Bump its version. The version is a whole number, counted from the pack's release history, so nobody edits it by hand.
  - Zip `pack.toml` and `global/**` reproducibly: files sorted, fixed timestamps. `sources/` stays out.
  - Record the SHA-256 and size.
- **Release:** create one GitHub release, for example `packs-2026-10-01.1`, holding the changed zips and a new `catalogue.json`.
  - The catalogue lists every pack's latest version, with its URL wherever that zip lives.
  - Older releases stay, so any old catalogue still resolves.
- **The clients' one fixed URL:**
  `https://github.com/<owner>/gs_studio_assets/releases/latest/download/catalogue.json`
  A direct download like this doesn't count against the GitHub API's rate limit of 60 calls an hour without a token.

`catalogue.json` (sketch):

```json
{
  "format": 1,
  "generated": "2026-10-01T12:00:00Z",
  "packs": [
    {
      "id": "ojandhaart",
      "title": "Ojandhaart (Hinterwilds)",
      "kind": "area",
      "version": 3,
      "format": 1,
      "size": 26214400,
      "sha256": "...",
      "url": "https://github.com/<owner>/gs_studio_assets/releases/download/packs-2026-10-01.1/ojandhaart-3.zip",
      "rooms": [7503118, 7503301, 7503307],
      "credits": ["Nisugi"],
      "license": "..."
    }
  ]
}
```

`rooms` holds the room uids its scenes bind to, the same number that leads a
scene file's name. With it, a client knows which pack covers the room a
character is standing in.

## 7. The clients

**Both clients:**
- Fetch the catalogue over HTTPS; no token needed.
- Verify every zip's SHA-256 before unpacking, and install through `gs_field::packs` into the pool.
- Nothing downloads on its own: every install or update is the user's choice, or an update setting they turned on. VellumFE's jinx client has the same rule.

**gs-studio:**
- The Packs panel (3.3b).
- On first run with an empty pool, it offers the starter packs.

**Hydra (Cena):**
- Doesn't use `gs_field` yet: the creature field is still ahead in its GUI plan (`plan/47`). When it does, it reads packs through `gs_field::packs`.
- Downloads with the `reqwest` it already has (`cena-platform`).
- Can offer "a scene pack covers this area" from the catalogue's `rooms`.
- The catalogue URL is a setting, so a fork or mirror can serve its own.

**Jinx compatibility (optional):** vellum-assets publishes Jinx manifests
that Lich's `;jinx` and VellumFE read. The asset repo could also publish a
Jinx manifest per pack. With VellumFE retiring, that's only worth doing if
Hydra wants the Jinx protocol (decision 8).

## 8. Phases

| Phase | What | Done when |
| --- | --- | --- |
| 0 | Answer section 9. Create `gs_studio_assets` with README, CONTRIBUTING, LICENSE and CODEOWNERS. | Decisions recorded in this file |
| 1 | gs_studio public: LICENSE, `ci.yml`, `release.yml`, the Linux build checked | A tag produces a release that Niffy downloads instead of an exe passed along by hand |
| 2 | `gs_field::packs`, the Packs panel, `--pack` and `--catalogue`. The first packs are cut from today's pool, and `release-packs.yml` publishes them. | A fresh gs-studio installs `ojandhaart` and the skin packs from the catalogue, and an update replaces files the user hasn't changed |
| 3 | `--validate` and `--sky` (Rust port); depth in CI with runner timings measured; the Backdrops view in the Scenery tab | A backdrop processed on a runner matches one processed locally |
| 4 | Issue forms, `submission.yml` (process and propose), the bot app, `pr-check.yml`, the ruleset. Backdrops and props first, then skins. | A test submission goes from issue to merged pack with nobody touching files |
| 5 | `rebuild.yml`; export-for-submission buttons | A rig change produces a PR with the rebuilt skins |
| 6 | Hydra installs packs | Hydra offers the pack for the area a character stands in |
| later | Creature art kind; stage-shot previews rendered without a GPU (wgpu on WARP or lavapipe, untested); a Discord route for people without GitHub (vellum-assets deferred it too); signed exe; depth in the app (ONNX) | — |

## 9. Decisions for the owner

1. **Owner account:** `Nisugi` or `elanthia-online`? Pushes to elanthia-online are broadcast on its Discord webhook.
2. **Art licence**, for example CC BY-NC-SA 4.0, CC BY 4.0, or "use in gs-studio and Hydra only". Also a fan-content notice: GemStone IV and its creature names belong to Simutronics.
3. **Depth model on the runners:**
   - Depth Anything V2 Large: finer detail, but its licence is non-commercial (CC BY-NC 4.0).
   - Small: Apache-2.0, smoother.
   
   Today's maps were made with Large.
4. **Pack size and budget:** one pack per area, per form's skins, per region's creatures? What's the largest size a pack may reach?
5. **Spam gate:** process every submission as it arrives (as vellum-assets does), or wait for a collaborator to add an `ok-to-process` label to a first-time submitter's issue? It's a public repo, and anyone can open issues.
6. **Where the catalogue lives:** Releases (recommended, one mechanism) or GitHub Pages like vellum-assets.
7. **Puppet textures:** they're built into `gs_field` today: the humanoid and lurk sheets plus 20 creature skins, about 29 MB in every binary, Android's included.
   - Move the skins into packs and keep only the plain family textures built in, as the "art ships as packs" rule suggests?
   - Or keep them built in?
8. **vellum-assets:** keep its UI categories there, or fold them into `gs_studio_assets`? And does Hydra want Jinx manifests?
9. **Reviewers:** who is in `CODEOWNERS`, and how many approvals does a PR need?
10. **Unsigned exe:** OK for now, with Windows' SmartScreen warning on first run?
