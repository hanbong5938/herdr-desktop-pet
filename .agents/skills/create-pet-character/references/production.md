# Production commands and data contracts

All scripts live in `.agents/skills/create-pet-character/scripts/` and take
`--run RUN`; everything character-specific comes from `RUN/run.json` (shape in
[SKILL.md](../SKILL.md#run-configuration), validation in `scripts/run_config.py`).
Commands below run from the repository root with
`SKILL=.agents/skills/create-pet-character`.

Upstream line references are to DAEMONLET commit
`1f5b1560162e4370368b0ab55c32c3271e5921c0` (see [UPSTREAM.md](../UPSTREAM.md)):
`skills/create-pet-character/…` for skill files and `scripts/characters/…` for the
creator runtime.

## Environment

| Component | Requirement |
| --- | --- |
| Pipeline Python | Python 3.9+ with numpy and Pillow (for example `/usr/bin/python3` on macOS). |
| Matting Python | Separate environment with numpy, Pillow, onnxruntime, scipy and optional pymatting; runs only `prepare-sources.py matte`. |
| Node | `tools.node` or `node` on PATH; runs the DAEMONLET runtime's See-through workflow/profile modules and PSD exporter. |
| See-through ComfyUI | `endpoints.seeThrough` (normally port 8188) with the nodes listed in `external-dependencies.json`. |
| Qwen-Image-2.1 ComfyUI | `endpoints.qwen` (normally port 8189): separate instance, `--disable-all-custom-nodes --disable-api-nodes`, offline Hugging Face environment, the three model files and core nodes `TextEncodeQwenImage21`, `QwenImage21Cache`. |
| DAEMONLET runtime | `tools.runtime`: creator runtime root containing `scripts/characters` (`build-source-models.py`, `lib/*`, `source_eye_blinks.py`, `export-model-psd.mjs`) and the See-through helpers in `scripts/`. Used as a library only; its Daemonlet packaging commands are not part of this pipeline. |
| herdr repository | `tools.adapter` (`tools/adapt-rig-source.py`), `tools.motionReference` (a complete v5 rig pack of this same character, manifest `name` = `character.label`, with `<pose>-motion.json`; see [SKILL.md](../SKILL.md#validation-and-delivery)), `tools.native` executable. |

`scripts/check-environment.py` probes both servers read-only (`--seethrough-url`,
`--qwen-url`, optional `--comfy-root` with the See-through ComfyUI's Python,
`--vram-gib`). It never installs, uploads or submits.

### See-through installation and models

When the user approves node installation, install the official
[ComfyUI-See-through](https://github.com/jtydhr88/ComfyUI-See-through) node pack in
the confirmed See-through ComfyUI through its Manager or manually into that
installation's `custom_nodes` with that installation's Python, then restart. Inspect
an existing installation first and do not auto-update it. Compare the compatible
commit, required nodes and `/object_info` from `external-dependencies.json`, and fix
import errors before continuing.

With `auto_download=true` the official loaders can download LayerDiff3D and Marigold
on first load. Use that only inside an approved model-preparation step and record
the actual snapshots; afterwards production keeps `auto_download=false`. Never put
weights in a pack. Node installation success and model-terms review are separate.

### Qwen-Image-2.1 instance

Keep the Qwen server a separate ComfyUI checkout (recorded commit
`8d534945ebd53cff61e8def81757c6a6c1b9cf2d`) bound to `127.0.0.1:8189`, started with
`--disable-auto-launch --disable-all-custom-nodes --disable-api-nodes` and
`HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1`. Model files:
`diffusion_models/qwen_image_2.1_bf16.safetensors`,
`text_encoders/qwen3vl_8b_bf16.safetensors` (CLIPLoader type `qwen_image`) and
`vae/qwen_image_2.1_vae_bf16.safetensors`. The reference graph is
[`workflows/qwen-image21-two-reference.json`](../workflows/qwen-image21-two-reference.json);
`scripts/qwen_local.py` builds the same graph and is the only client.

## Run layout

```text
RUN/run.json
RUN/identity/attempts/<id>/…                    raw Qwen attempts + provenance
RUN/identity/corrected-<seed>.png(.provenance.json)
RUN/generation/<pose>-v1.png(.provenance.json)
RUN/generation/attempts/<pose>-v1-<uuid>/…
RUN/source-index.json                           derived index of accepted pose sources
RUN/poses/<pose>/inputs/master.png              1280² RGBA matte
RUN/poses/<pose>/inputs/matting.json
RUN/poses/<pose>/inputs/matte-review.json       operator rejection of the published matte
RUN/poses/<pose>/raw/matting/<attempt>/         model/refined alpha, pocket-cleared.png, reviews
RUN/poses/<pose>/native/                        See-through parts + generation.json
RUN/poses/<pose>/plan/geometry.json
RUN/poses/<pose>/plan/expression-plan.json
RUN/poses/<pose>/plan/own-expression-request.json
RUN/poses/<pose>/plan/masks/*.png
RUN/poses/<pose>/raw/own-expression-G0.png
RUN/poses/<pose>/raw/expression-attempts/<pose>-<uuid>/…   raw Qwen attempts + provenance
RUN/poses/<pose>/raw/expression-review.json     operator rejection of the published sheet
RUN/poses/<pose>/raw/rejected/<sha256[:16]>/    archived rejected sheet, request, review
RUN/poses/<pose>/<round>/                       source rig round (for example R1)
RUN/native-models/<pose>/                       model.psd, rig-overrides.json, motion.json,
                                                provenance.json, own-source-record.json
RUN/superseded/<pose>/<n>-<masterSha[:16]>/     superseded.json + archived pose/ and native-model/
RUN/pack/                                       staged herdr v5 pack
RUN/reviews/<name>/                             native captures
```

Raw outputs, attempts, provenance and reviewed geometry are created exclusively and
never overwritten; a failed attempt stays on disk. Only derived indexes such as
`source-index.json` are atomically replaced. This replaces upstream
`skills/create-pet-character/references/production.md:26-37`
(`outputs/characters/<run>` inside the Daemonlet checkout and `plan/poses.json`).

## Stages

Steps 2–7 accept `--poses waiting,writing,head-tap` for the pilot; `all` selects the
ten poses. Staging and native review need all ten.

### 1. Identity correction

```sh
python3 "$SKILL/scripts/identity-correct.py" --run RUN
```

Options: `--seed N` (default `seeds.identity`), `--resolution 768`, `--feather PX`
(Gaussian feather, default 6, 0–64). This is the only step that uses the external
identity image: image1 = authority `headCrop`, image2 = `identityReference.faceCrop`,
prompt `prompts.identityCorrect` (with optional `{faceEditRect}` in headCrop-local
coordinates). The result is composited only inside `authority.faceEditRect` (see
the procedure below). Writes `identity/attempts/<id>/…`,
`identity/corrected-<seed>.png`, its provenance and
`identity/corrected-<seed>-review.png` (identity crop, before, after). Review the full
image and enlarged face against the identity reference; if accepted, point
`run.json` `authority.path` and `sha256` at the corrected file manually. All later
steps verify that hash.

### 2. Pose sources

```sh
python3 "$SKILL/scripts/generate-sources.py" poses --run RUN --poses waiting,writing,head-tap
```

The source pose (`authority.sourcePose`) is copied byte-exactly, not regenerated.
Every other pose is a new complete 1280² illustration: image1 = (corrected)
authority, image2 = authority `headCrop` (face identity only), prompt =
`prompts.pose` with that pose's `poseIntents` entry, seed derived from
`seeds.poses`. Outputs that duplicate the authority or another pose are rejected.
Each pose gets `generation/<pose>-v1.png` + provenance; `source-index.json` lists
accepted sources.

### 3. Matte

```sh
<matting-env>/bin/python "$SKILL/scripts/prepare-sources.py" matte --run RUN --poses waiting,writing,head-tap
```

isnet-anime (`tools.mattingModel`) on onnxruntime CPU plus pymatting edge refinement
(default on) produces `poses/<pose>/inputs/master.png` (1280² RGBA; artwork stays on
the original square) and `matting.json` (source/model hashes, environment versions,
crop annotation, `status: pending_visual_review`). Inspect the white/dark review
composites before decomposition. See
[reference-preparation.md](reference-preparation.md#subject-alpha).

**Background-pocket suppression** (`inputs.methodVersion: 2`). The matte model keeps
enclosed pockets of plain generation background (between hair strands, between hair
and arm, between arm and body) at partial alpha (~0.3–0.6) with near-white RGB;
See-through then files them under `back hair` and they render as grey bands on dark
desktops. After the model/refined alpha, the matte takes the per-channel median of
the raw image's one-pixel border as the background colour (at least 75% of border
pixels must lie within the tolerance, otherwise the matte fails: the generation
background is not plain) and sets alpha to 0 where the raw RGB is within 16 (max
per-channel difference) of it **and** alpha is below 0.98 (uint8 < 250). Opaque light
garment interiors (alpha ≥ 0.98, bounded by line art) and all other pixels keep the
refined alpha and colour; cleared pixels become transparent black like the rest of
the background. `matting.json` records `backgroundPocketSuppression`
(`backgroundRgb`, `borderUniformFraction`, `tolerance`, `alphaBelow`,
`pixelsCleared`, `alphaMassCleared`, `components` ≥ 64 px with `[x0,y0,x1,y1]` bounds
and median alpha, and the `pocket-cleared.png` mask hash in the attempt). Most
cleared pixels are near-zero-alpha background haze around the silhouette; check the
components with high median alpha against the dark composite. Pockets the model
matted fully opaque (alpha ≥ 0.98) are deliberately left alone. A pose matted with an
older `methodVersion` is never silently re-matted: reject it and supersede it.

**Rejecting a matte.** Write `poses/<pose>/inputs/matte-review.json`:

```json
{"decision": "rejected", "masterSha256": "<sha256 of the published inputs/master.png>",
 "reason": "grey background pocket in back hair at x876-930 y408-547"}
```

While it exists, a plain `matte` run for that pose refuses to continue; do not run
later stages on the rejected matte. Then run

```sh
<matting-env>/bin/python "$SKILL/scripts/prepare-sources.py" matte --run RUN --poses happy --supersede-rejected
```

Only when the review's `masterSha256` equals the current master and `matting.json`
agrees, this creates `RUN/superseded/<pose>/<n>-<masterSha[:16]>/` (n = 1, 2, … per
pose; refused if it exists), writes `superseded.json` there first (n, reason, the
review and its hash, master/matting hashes, old `methodVersion`, and per moved tree
`from`, `to` and the SHA-256 of every file), then renames the whole `poses/<pose>/`
tree to `…/pose/` (leaving a new empty `poses/<pose>/`) and `native-models/<pose>/`,
if present, to `…/native-model/`. Nothing is deleted or overwritten; an interrupted
archival is finished (after re-verifying the file hashes) by the next `matte` run.
The pose is then matted afresh with the current method, and every downstream stage
runs fresh for it: decompose, geometry, expressions, `build --round R1` (the round
and `native-models/<pose>/` are free again) and a new `assemble-pack.py stage
--output` directory. Paths written inside archived records still name their
pre-archive location; `superseded.json` maps them. Poses without a review are left
to the ordinary exact-hash resume.

### 4. Decompose

```sh
python3 "$SKILL/scripts/prepare-sources.py" decompose --run RUN --poses waiting,writing,head-tap
```

Uploads one master at a time to `endpoints.seeThrough` only when both ComfyUI
servers are idle: 1280px, 30 steps, cached tag embeddings, alpha preserved through
`JoinImageWithAlpha`, seed `seeds.seeThrough` + pose index. Stores parts,
`<pose>_layers.json` and `generation.json` (source hash, graph, workflow/profile
hashes, VRAM figure, prompt ID) in `poses/<pose>/native/`.

Required semantic parts: face, hair, eyes, mouth, hands, `topwear` and one own
lower-body plate (`bottomwear`, `legwear` or `body`; a camisole and shorts may come
back as a single `topwear`). A missing part fails the pose with the raw attempt kept
under `poses/<pose>/raw/decomposition/<attempt>/`. A timed-out or failed attempt is
recovered without new inference by
`decompose --run RUN --poses <pose> --recover-attempt <attempt>` once the queue is
idle; a re-evaluation that differs from the stored `run-summary.json` is written as
`run-summary-recovered-<uuid>.json` and referenced from `generation.json`.

Group offload: enable on both loaders only when the See-through device's **total**
memory is <=12 GiB, disable both above 12 GiB. Use total capacity, never free
memory. The runtime's automatic selection accepts only a single CUDA device; on
Apple Silicon (MPS, unified memory) the capacity comes from `run.json`
`seeThrough.vramGiB` (default 64, as in the proven run, so offload is off). <=8 GiB
is discouraged (layer resolution 1024, depth 720). Check the loaders' offload mode
in the inference log. `workflows/seethrough-api.json` remains a static example with
offload off. This replaces the CUDA-only wording of upstream
`skills/create-pet-character/references/production.md:39-49`.

### 5. Geometry

```sh
python3 "$SKILL/scripts/build-rigs.py" geometry --run RUN --poses waiting,writing,head-tap
```

Measures the pose's own See-through parts and writes `plan/geometry.json`,
`plan/expression-plan.json` (face crop `source_canvas_crop` + master hash) and mask
candidates in `plan/masks/` (`sourceEyeMasks`, `sourceIrisMasks`,
`neutralMouthMask`). It fails instead of guessing when a part is missing, sides are
ambiguous, the nose or mouth includes jaw/hair, or eye pigment checks fail. Review
geometry and masks at magnified scale before expressions; reviewed geometry is never
overwritten. Extraction rules:
[visual-review.md](visual-review.md#eye-lid-and-mouth-extraction).

### 6. Expressions

```sh
python3 "$SKILL/scripts/generate-sources.py" expressions --run RUN --poses waiting,writing,head-tap
```

Builds a sheet from two copies of the pose's own face crop (`source_canvas_crop`)
and edits it at resolution 768: image1 = sheet, image2 = the same pose's own head
crop, prompt `prompts.expression`, seed `seeds.expressions` + pose index (+ 1000 per
archived rejection, below). Left panel: closed eyes and small open mouth; right
panel: open eyes and gentle smile. The input sheet's aspect is 2·crop width / crop
height, not 2:1 (the proven crops gave 1.54–2.30). The output must be exactly the
`TextEncodeQwenImage21` canvas for that input sheet (aspect kept, sides rounded to
multiples of 32, e.g. 960×608 or 1152×512) with an even width, so it splits into
two equal panels; otherwise the attempt is preserved and not published. Writes
`raw/own-expression-G0.png` and `plan/own-expression-request.json` (crop, master
hash, sheet hashes, prompt ID, seed, models, `expressionSheetCheck`).

**Rejecting and regenerating a sheet.** The published names are fixed (`build` reads
`raw/own-expression-G0.png`), so a sheet is never edited or deleted in place. To
reject it, the operator writes `poses/<pose>/raw/expression-review.json`:

```json
{"decision": "rejected", "sha256": "<SHA-256 of raw/own-expression-G0.png>",
 "reason": "closed-eye panel misregistered", "source": "build-registration",
 "error": "<the build-rigs.py error message, verbatim>", "round": "R1"}
```

`source` is `visual-review` (default; inspection found a defect) or
`build-registration` (`build-rigs.py build` failed with `Misregistered expression
panel …` or `No closed-lid lash contrast …`; `error` must quote that message).
Then:

```sh
python3 "$SKILL/scripts/generate-sources.py" expressions --run RUN --poses failed --supersede-rejected
```

Only when the review's `sha256` matches the verified published sheet, this creates
`raw/rejected/<sha256[:16]>/` (refused if it exists), writes `superseded.json` there
(rejection number N, review, seed, prompt ID, the planned moves with hashes),
renames `expression-review.json`, `plan/own-expression-request.json` and
`raw/own-expression-G0.png` into it, and records the rejection as
`visual-review.json` in the superseded Qwen attempt so the attempt guard accepts
it. It then generates a new attempt with seed `seeds.expressions + pose index +
1000·N` and publishes a new G0 and request with the same fields plus
`regeneration: N` and `supersedes` (archive, old sheet hash, prompt ID, seed,
reason). Nothing is deleted or overwritten; an interrupted archival is finished on
the next run. Without the flag a rejected sheet stops the command; poses without a
review are retained. The failed build's round directory stays as evidence, so
rebuild that pose with a new round name (for example `--round R2`).

### 7. Build

```sh
python3 "$SKILL/scripts/build-rigs.py" build --run RUN --poses waiting,writing,head-tap --round R1
```

Builds the source rig round `poses/<pose>/R1/` with the runtime's
`build-source-models.py`, patched by `scripts/expression_fixes.py` (similarity
registration, hue-independent closed-lid extraction, neutral-mouth-width-scaled
mouth extraction kept inside the own face, blink regularization), requires reviewed
eye/iris/mouth masks, exports the PSD with the runtime's exporter, converts it with
`tools.adapter` to 1024² and
writes `native-models/<pose>/` (`model.psd`, `rig-overrides.json`, `motion.json`,
`provenance.json`, `own-source-record.json` with the fix parameters and
measurements under `fixes`). Motion follows
[motion-authoring.md](motion-authoring.md#herdr-native-motion-constraints). Any
rebuild needs a new round name.

### 8–10. Stage, review, deliver

```sh
python3 "$SKILL/scripts/assemble-pack.py" stage --run RUN
python3 tools/character-pack.py validate --native BIN --path RUN/pack
python3 "$SKILL/scripts/review-native.py" --native BIN --path RUN/pack --output RUN/reviews/full-1
```

`stage` assembles the ten `native-models/` into a herdr v5 rig pack (default
`RUN/pack`, or `--output DIR`): the v5 entry from the `tools.motionReference` pack's
template, that pack's license/owner records carried verbatim with a run notice,
`source-record.json`, regenerated inventory and a passing native validation before
the directory appears. It refuses a reference pack whose manifest `name` is not
`character.label`; a new character first needs its own reference working copy with
its own verified rights records. Pack rules, preview and packaging are in
[character-creator](../../character-creator/SKILL.md) (`tools/character-pack.py
validate`, `preview`, `review`, `package`; native `pack preview --pose <id>`).

`review-native.py` writes `review.json`, per-pose neutral, hit-region and
clip-timeline captures (entry, middle, near-expiry, low-blink, mouth and head-angle
keys), unforced phase and reaction captures, and light/dark contact sheets into a
new directory. Captures are evidence for human inspection, not approval.

To replace the bundled default only:

```sh
python3 "$SKILL/scripts/assemble-pack.py" cutover --run RUN --pack RUN/pack \
  --builtin REPO/assets/rubelia-default --native BIN
python3 "$SKILL/scripts/assemble-pack.py" thumbnail --run RUN --replace \
  --capture RUN/reviews/full-1/waiting-native.png --output REPO/assets/rubelia-thumbnail.png
```

`cutover` revalidates natively first, backs up the current builtin under RUN, swaps
the directory, and restores the previous builtin on any failure. `thumbnail` derives
the menu thumbnail from a native waiting-pose capture recorded by `review-native.py`
(alpha-bound crop, aspect fit). For any other character, package with
`tools/character-pack.py package` and import with the native `pack import` route.
Together these replace upstream
`skills/create-pet-character/references/production.md:65-98` (`models.json`, click
variants, the Daemonlet capture/payload/export tools and app-UI import; full list in
[UPSTREAM.md](../UPSTREAM.md)).

## Local Qwen edit procedure (fix 1)

Replaces the built-in image tool sheet edit of upstream
`skills/create-pet-character/references/production.md:57-63` and DAEMONLET
`scripts/characters/finish-source-models.py:14,24-25,32-34` (2048² green atlas
handed to a hosted built-in image tool, chroma-keyed adoption of whole regenerated
parts).

1. **Graph.** `LoadImage`×2 → `JoinImageWithAlpha`; `UNETLoader`, `CLIPLoader`
   (`qwen_image`), `VAELoader`; `TextEncodeQwenImage21` with `images.image_1` (own
   crop) and `images.image_2` (reference crop); `QwenImage21Cache`; `KSampler` 25
   steps, cfg 1, `euler`/`simple`, denoise 1; `VAEDecode`; `SaveImage` (node 19).
   Before upload, require the three model files, the `qwen_image` CLIP type, the
   `images` autogrow schema and idle queues. Never cancel or interrupt other jobs.
2. **Resolution and crop sizing.** `TextEncodeQwenImage21` renders at about
   `resolution²` pixels (sides rounded to multiples of 32, aspect kept). Face edits
   therefore run at 768 on a small crop, never on the full 1280 canvas, so the face
   is upsampled rather than downsampled: choose `authority.headCrop` to hold the
   whole head with hair and ornaments plus a margin (the proven run used 350×345 px)
   and `faceEditRect` tight around skin and features. The expression crop comes from
   geometry (own face bounds widened to ears, eyebrows and lip, never torso). The
   decoded output is resized back to the crop with LANCZOS. Pose generation is the
   only 1280 call.
3. **Identity chain.** The external identity image enters once, as image2 of
   `identity-correct.py`. Its prompt states that image2 supplies face identity only;
   outfit, background, props and framing come from image1. After review the
   corrected image becomes the authority, so `generate-sources.py poses` carries
   identity via the authority head crop. Expression edits use the pose's own head
   crop as image2: an external identity image there would redraw the face and break
   neutral↔expression registration. Reject any output that imports outfit or props
   from a reference, and fix identity drift at the authority, not per pose.
4. **Mask composite.** Identity correction composites the generated crop back only
   inside an ellipse inscribed in `authority.faceEditRect` (inset by
   `ceil(feather)`, thresholded, Gaussian-feathered, clipped to the rectangle) and
   restores the original alpha. Expression panels are never pasted wholesale: the
   build registers each panel and extracts only eye, lid and mouth features from it.
5. **Preservation proof.** Identity provenance must record
   `outsideMaskRGBAExactlyUnchanged` and `alphaExactlyUnchanged` as true, zero
   `changedPixelsOutsideSupport` and a non-zero `changedPixels`; otherwise the
   attempt fails and is preserved. Every Qwen call records endpoint, prompt ID,
   seed, the full graph, model files, prompt and SHA-256 of every input/output. No
   manual face painting, donor-face transplant or fabricated provenance.

## Registration (fix 2)

Replaces DAEMONLET `scripts/characters/build-source-models.py:52-63`
(`register_panel`: translation-only search of ±12 px on the nose box). Qwen output
drifts in scale as well as position. `expression_fixes.py` registers both panels of
one Qwen render to the own neutral face (upstream metric divided by a
featureless-panel baseline gives the normalized score):

- **Panel 1 (open eyes):** similarity transform (uniform scale + translation) on a
  nose/eye/eyebrow patch, front hair and mouth rows excluded, at least 200 pixels.
  Scale searched 0.82–1.18 and accepted within 1 ± 0.10, translation searched
  ±24 px (reaching the limit rejects), normalized score ≤ 0.35.
- **Panel 0 (closed eyes; eye apertures and lashes also excluded):** reuses panel
  1's scale and searches translation only within ±14 px of panel 1's (reaching the
  limit rejects); normalized score at that scale ≤ 0.55. Its scale is verified by a
  free panel 0 → panel 1 similarity match on the shared sheet content (crop inset
  24 px, eyes and mouth excluded with 16 px padding), which must be within 1 ± 0.03.

Calibration on the 10 proven sheets: panel-1 scale 0.990–1.055, score 0.104–0.174;
panel-0 offset ≤ 10.5 px, score 0.059–0.371; panel 0 → 1 scale 0.996–1.005.
Transforms, scores and limits are recorded in `own-source-record.json`
(`fixes.registration`). A rejected panel is never forced into place or accepted
through a translation-only fallback: record the build error in
`expression-review.json` and regenerate the sheet with
`generate-sources.py expressions --supersede-rejected` (stage 6). See
[visual-review.md](visual-review.md#misregistration).

## Chroma key (fix 3)

Replaces DAEMONLET `scripts/characters/lib/assemble_finish_art.py:5-44`
(`chroma_matte`, exact `#00FF00` matte). No step paints parts on a green sheet:
identity edits are mask-composited on the original pixels, expression features are
extracted from registered panels, and subject alpha comes from isnet-anime matting
([reference-preparation.md](reference-preparation.md#subject-alpha)).

## Blink stability (fix 7)

The runtime measures per-pose lid profiles (49-sample `upper`, `lower` and `closed`
curves from DAEMONLET `scripts/characters/source_eye_blinks.py:29-45`, used raw by
`scripts/characters/build-source-models.py:152` `blink_profiles`). Sample jitter shows
up as mesh wobble at low blink values. `expression_fixes.py` applies a sampled
Gaussian (sigma 3 samples, radius 9, edge-replicated ends) to each curve
independently, changing rig geometry only — no RGB or alpha edits — and records per
eye the original and smoothed curves and `maxRigCoordinateShiftPx` with
`rasterPixelsChanged: false`. This reproduces the proven run's regularization
record. A shift beyond a pixel or two means the measurement, not the smoothing,
needs review.

## Completion record

Keep authority, identity, pose, master, part and sheet hashes; requested and actual
generation/decomposition settings; selection and retry history; per-pose motion;
native review captures; delivery result; and unresolved rights. Model, original
character and input-image licenses are separate. Qwen-Image-2.1's recorded terms
(Qwen RESEARCH LICENSE AGREEMENT; private non-commercial research/evaluation) do not
grant redistribution of a character; never infer rights from model-output ownership
terms. Weights stay in the user's environment and never enter a pack.
