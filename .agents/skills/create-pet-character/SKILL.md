---
name: create-pet-character
description: Produce a herdr v5 ten-pose rig character (one independent illustration, See-through layer set, own expressions and motion per pose) with a private local ComfyUI Qwen-Image-2.1 server and ComfyUI See-through, then stage, natively validate and review the pack. Use for new herdr characters or for replacing the bundled default's artwork from an approved illustration.
---

# Herdr ten-pose character production (local Qwen-Image-2.1 + See-through)

This skill turns one approved character illustration into a herdr v5 rig pack: ten
independent poses, each with its own artwork, See-through parts, eye/mouth
expressions, geometry and motion. Every image generation or edit runs on a private
local ComfyUI Qwen-Image-2.1 server; decomposition runs on a local ComfyUI
See-through server. Native pack rules — manifest, runtime, capabilities, limits,
validation, preview, packaging and import — belong to
[character-creator](../character-creator/SKILL.md); read it before staging and do
not restate or bypass it here. Fork provenance and the upstream changes are listed
in [UPSTREAM.md](UPSTREAM.md).

Read [production.md](references/production.md) for executable commands and data
contracts, [visual-review.md](references/visual-review.md) before the first
decomposition, [reference-preparation.md](references/reference-preparation.md) when
the source needs cleanup or matting, and
[motion-authoring.md](references/motion-authoring.md) before any motion work.

## Inputs

Reuse answers already given. Ask for:

- **Authority illustration**: the approved 1280×1280 illustration that defines
  outfit, body, style and (after identity correction) the face. Inspect it with the
  reference preparation step below. Every Qwen call in this pipeline is a
  two-reference edit, so the scripts do not invent an authority from a text
  description; without one, ask the user to supply or approve an illustration first.
- **Identity reference**: the face identity authority (any size) and its face crop.
  It may be the authority itself. It supplies face identity only, never outfit,
  background or props.
- **Framing**: production framing is thigh-up with an unusually large head (see
  [reference-preparation.md](references/reference-preparation.md)). Preserve a
  supplied thigh-up authority; for a full-body authority, explain that generated
  poses use thigh-up framing so the face has enough pixels for See-through and Qwen,
  and confirm before generating. Apply one framing to every pose.
- **Hosts and tools**: the host running both ComfyUI servers, the See-through
  ComfyUI URL (normally `http://127.0.0.1:8188`) and root, the Qwen-Image-2.1 ComfyUI
  URL (normally `http://127.0.0.1:8189`), the matting environment's Python and
  `isnet-anime.onnx` path, the DAEMONLET creator runtime root, and the herdr native
  executable. Do not reuse another person's paths or addresses.
- **Character**: display name, unique pack ID, props, intended use/distribution, and
  the rights status of the authority and identity images.
- **Pose intents**: one short intent per pose except the source pose (normally
  `waiting`, which copies the authority exactly). Explain the poses first.

Keep source-image provenance and note unresolved input/model rights.
`external-dependencies.json` lists the external tools, model files and recorded
license status; Qwen-Image-2.1's recorded terms restrict use to private
non-commercial research/evaluation. Check readiness early, before generating poses.
When setup is needed, follow **Dependency setup with approval** below; a character
request alone is not permission to install, update or download anything.

### Explain the ten poses

In the user's language, explain all ten poses and propose character-appropriate
intents before generating. The examples are suggestions, not fixed artwork.

| Pose | Purpose and example artwork |
| --- | --- |
| `waiting` | Normal waiting state; a relaxed neutral stance. |
| `writing` | Work in progress; concentrating or writing. |
| `failed` | Work failed; concern, surprise or frustration. |
| `cancelled` | Work was cancelled; pausing or acknowledging the interruption. |
| `disconnected` | Connection is unavailable; looking puzzled or waiting for reconnection. |
| `bored` | Prolonged inactivity; fidgeting, yawning or another idle activity. |
| `happy` | A happy reaction; smiling or celebrating. |
| `head-tap` | Head-click reaction; a brief response to a click on the head. |
| `torso-tap` | Torso-click reaction; a brief response to a click on the body. |
| `head-pet` | Head-petting reaction; responding to a stroking gesture, distinct from a click. |

Herdr v5 binds exactly these ten semantic poses, each to a different model, and has
no click-variant selection. This skill therefore produces exactly ten models, one
illustration per pose; do not offer alternative click illustrations or extra
variants. Keep pose intents free of props, text and anything that hides the face or
eyes, because the face must stay measurable in every pose.

## Dependency setup with approval

If See-through, the Qwen-Image-2.1 instance, matting or required models/packages are
missing, incompatible or incomplete, offer to install or repair them instead of
stopping at instructions. Reuse an explicit setup authorization already given for
the same target and scope.

1. **Inspect first.** Confirm the actual host, both ComfyUI roots and Pythons, URLs
   and device. Run `scripts/check-environment.py` (read-only): it probes the
   See-through server's nodes and device, and the Qwen server's `/system_stats`,
   required nodes (`TextEncodeQwenImage21`, `QwenImage21Cache`) and the three model
   files; add `--comfy-root` when running with the See-through ComfyUI's Python to
   check its packages. Distinguish a missing node pack, missing weights, an
   incomplete snapshot and a package incompatibility.
2. **Present a concrete plan and ask whether to perform it.** Identify target paths,
   official sources and pinned revisions from `external-dependencies.json`, missing
   components, download/disk estimates (or what is unknown) and the recorded model
   terms. The Qwen server is a separate ComfyUI instance on port 8189 started with
   `--disable-all-custom-nodes --disable-api-nodes`; do not add custom nodes to it
   or merge it with the See-through instance. Disclose package changes,
   configuration edits, cache repairs and any server restart. Model installation
   approval covers the named files; it is not a grant of model rights.
3. **Execute the approved scope.** Use the selected installation's Python, pin and
   record actual revisions/hashes, preserve the prior package list and changed
   configuration outside Git, and download only approved files. Stop and ask about
   a concrete scope change (another model, unrelated package, different host or
   unapproved restart).
4. **Protect running work.** Check both queues before setup, restarts or test jobs.
   Wait for existing jobs; never cancel or interrupt them. If installation fails,
   report partial changes and the blocker; do not run repeated broad upgrades or
   delete shared caches.
5. **Verify and resume.** After an approved restart rerun the environment check and
   inspect import logs. Use the first pilot pose for one actual Qwen edit and one
   actual See-through decomposition, checking the loaders' offload mode in the logs.
   A node or model listing is not inference proof.

Keep setup records, private paths and logs in the run outside Git and distributable
packs. `check-environment.py` stays read-only regardless of approvals.

## Reference suitability and preparation

Inspect the supplied images before generation. Distinguish an identity/design
reference from a production-ready single-pose illustration. Character sheets, opaque
backgrounds, heavy lighting, fine overlapping hair and rough edges can require
preparation; explain the specific issue and proposed correction instead of rejecting
the reference or silently redesigning it. A thigh-up crop is valid framing, not
missing anatomy to repair.

Read [reference-preparation.md](references/reference-preparation.md) for cleanup,
sheet extraction, matting and framing. Preserve face, defining features, clothing,
colors and the selected pose; preserve originals and show the prepared candidate
for selection before generating poses. Normalize orientation/color/canvas with
`scripts/prepare-reference.py`; it neither removes backgrounds nor reconstructs
hidden parts. Subject alpha for pose masters comes from `prepare-sources.py matte`
(isnet-anime), not from a painted or green background.

## Run configuration

Create a fresh private run directory outside Git and write `RUN/run.json`. Every
script reads only this file for character, inputs, prompts, endpoints, seeds and
tools; none hard-code paths or character names. Relative paths resolve against RUN.

```json
{
  "schemaVersion": 1,
  "character": {"id": "example-character", "label": "Example Character"},
  "canvas": 1280,
  "poses": ["waiting", "writing", "failed", "cancelled", "disconnected", "bored", "happy", "head-tap", "torso-tap", "head-pet"],
  "authority": {
    "path": "inputs/authority.png",
    "sha256": "<sha256 of inputs/authority.png>",
    "headCrop": [470, 60, 830, 410],
    "faceEditRect": [540, 150, 760, 380],
    "sourcePose": "waiting"
  },
  "identityReference": {
    "path": "inputs/identity.png",
    "sha256": "<sha256 of inputs/identity.png>",
    "faceCrop": [400, 90, 610, 280]
  },
  "prompts": {
    "identityCorrect": "<image1> is a close crop of the current pose; <image2> is the sole face identity reference. Redraw only the face inside {faceEditRect} to match <image2>; keep head tilt, hair, hands and framing of <image1>.",
    "pose": "Generate ONE new complete illustration of the SAME character from <image1> in a different whole-body pose. <image2> supplies only face identity. Keep an unusually large head including entire hair through upper thighs filling nearly the entire 1280-square height. Pose intent: {intent}. Plain opaque white background, no text, no props.",
    "poseIntents": {
      "writing": "focused downward gaze with both eyes visibly open, hands near the waist",
      "failed": "concerned restrained expression, head slightly lowered",
      "cancelled": "gentle resigned expression with one relaxed stopping gesture",
      "disconnected": "puzzled small head tilt and one questioning hand gesture",
      "bored": "subdued sleepy mood, both eyes visibly open",
      "happy": "warm pleased smile, one hand near chest without covering the face",
      "head-tap": "surprised shy head tilt, one hand near hair never crossing the face",
      "torso-tap": "mild playful surprise, one hand at side or chest",
      "head-pet": "content gentle smile, one hand near cheek or hair without covering either eye"
    },
    "expression": "Edit <image1> into EXACTLY TWO equal side-by-side panels of the same face. <image2> is a crop of this same pose's head, reference only. LEFT: both eyes completely shut, small open mouth. RIGHT: eyes open, gentle smile. No text, no third panel."
  },
  "endpoints": {"qwen": "http://127.0.0.1:8189", "seeThrough": "http://127.0.0.1:8188"},
  "seeds": {"identity": 1, "poses": 2, "expressions": 3, "seeThrough": 42013},
  "seeThrough": {"vramGiB": 64},
  "tools": {
    "runtime": "/abs/path/DAEMONLET",
    "mattingModel": "/abs/path/models/matting/isnet-anime.onnx",
    "adapter": "/abs/path/herdr-desktop-pet/tools/adapt-rig-source.py",
    "motionReference": "/abs/path/work/example-character-reference",
    "native": "/abs/path/herdr-desktop-pet/native/target/release/herdr-desktop-pet",
    "node": "/opt/homebrew/bin/node"
  }
}
```

- `canvas` is 1280 and `poses` lists exactly the ten IDs. Boxes are
  `[x0, y0, x1, y1]` pixels: `authority.headCrop` and `authority.faceEditRect` in
  the 1280 authority canvas, `identityReference.faceCrop` in the identity image.
- `authority.faceEditRect` is required by `identity-correct.py` and must lie inside
  `headCrop`; it bounds the only pixels identity correction may change. An optional
  `{faceEditRect}` placeholder in `prompts.identityCorrect` is replaced with that
  rectangle in headCrop-local coordinates.
- `authority.sourcePose` (normally `waiting`) is copied exactly instead of
  regenerated, so `prompts.poseIntents` needs every other pose. `prompts.pose` must
  contain `{intent}`.
- `tools.runtime` is the DAEMONLET creator runtime root containing
  `scripts/characters`; `tools.native` is optional for generation but needed by
  review and pack assembly; optional `tools.node` defaults to `node` on PATH.
  Seeds are non-negative integers below 2^63.
- Optional `seeThrough.vramGiB` (default 64) is the See-through device's total
  memory used for the group-offload rule; set it explicitly, because MPS reports no
  CUDA VRAM for automatic detection.
- Optional `request` (`{"userStatement": "…", "date": "YYYY-MM-DD", "scope": "…"}`)
  records the user's verbatim request for this run; `assemble-pack.py stage` copies
  it into `source-record.json` `approval.request`. It is not a rights or
  publication grant.
- After identity correction, edit `authority.path`/`sha256` by hand to the accepted
  corrected file. Never edit hashes to make a changed file pass.

## Pipeline

Run commands from the repository root with `SKILL=.agents/skills/create-pet-character`
(`python3 "$SKILL/scripts/<script>" …`) using the pipeline Python (Python 3.9+,
numpy, Pillow), except `prepare-sources.py matte`, which runs with the matting
environment's Python. Every script refuses to overwrite raw outputs, attempts,
provenance or reviewed geometry, and requires an idle ComfyUI queue before
submitting.

| Script | Commands |
|---|---|
| `qwen_local.py` | importable module: ComfyUI client + Qwen-Image-2.1 two-reference graph |
| `identity-correct.py` | `--run RUN [--seed N] [--resolution 768] [--feather PX]` |
| `generate-sources.py` | `poses --run RUN [--poses all\|a,b]` ; `expressions --run RUN [--poses …] [--supersede-rejected]` |
| `prepare-sources.py` | `matte --run RUN [--poses …]` (run with matting-env python) ; `decompose --run RUN [--poses …]` |
| `build-rigs.py` | `geometry --run RUN --poses …` ; `build --run RUN --poses … --round R1` |
| `review-native.py` | `--native BIN --path PACK --output NEW_DIR` |
| `assemble-pack.py` | `stage --run RUN [--output DIR]` ; `cutover --run RUN --pack DIR --builtin REPO/assets/rubelia-default --native BIN` ; `thumbnail --run RUN --capture PNG --output PNG [--size WxH] [--replace]` (`--replace` is required when the output exists, e.g. the bundled thumbnail) |

Order (each step's output is reviewed before the next):

1. `identity-correct.py` → review `identity/corrected-<seed>.png` and its
   provenance → update `run.json` `authority` to the accepted file.
2. `generate-sources.py poses` → `generation/<pose>-v1.png`, `source-index.json`.
3. `prepare-sources.py matte` (matting-env Python) → `poses/<pose>/inputs/master.png`.
4. `prepare-sources.py decompose` → `poses/<pose>/native/`.
5. `build-rigs.py geometry` → `poses/<pose>/plan/geometry.json`, masks, expression plan.
6. `generate-sources.py expressions` → `poses/<pose>/raw/own-expression-G0.png`. A
   rejected sheet (`raw/expression-review.json`) is archived and regenerated with
   `--supersede-rejected` (production.md, stage 6).
7. `build-rigs.py build` → source round `poses/<pose>/R1/` and `native-models/<pose>/`
   (a rebuild after regeneration needs a new round name, e.g. `R2`).
8. `assemble-pack.py stage` → staged herdr v5 pack (`pack/` by default).
9. Native validation and preview per [character-creator](../character-creator/SKILL.md),
   then `review-native.py` → `reviews/<name>/`.
10. Only when replacing the bundled default: `assemble-pack.py cutover`, then
    `assemble-pack.py thumbnail` from a native waiting-pose capture. Otherwise
    package and import the staged pack with the character-creator commands.

## Production

Run the pilot first: take `waiting`, `writing` and `head-tap` through steps 2–7
(`--poses waiting,writing,head-tap`) and review sources, mattes, layers, geometry,
expression sheets and built rounds before expanding to the remaining poses. The
staged pack and native review need all ten models, so the native visual and motion
passes follow expansion. When a defect repeats across poses, stop expanding, find
the shared cause, repair one pilot and verify it before batch application. Use the
four separate review stages in [visual-review.md](references/visual-review.md).

Each pose gets its own body, head, eyes and mouth from its own illustration. Reuse
tools, not another pose's artwork or coordinates; never transplant a donor face or
paint a face by hand. Identity flows through one chain: `identity-correct.py` is
the only step that uses the external identity reference (image2) and composites
only inside `authority.faceEditRect`; the reviewed result becomes the authority, so
pose generation carries identity through the authority head crop; expression edits
use the pose's own head crop so their panels stay registered to that pose's
neutral face. See [production.md](references/production.md) for the exact
procedure and proofs.

See-through: 1280px, 30 steps and cached tag embeddings. **Enable group offload on
both loaders only when the selected device has <=12 GiB total memory; disable it on
both above 12 GiB.** Use the See-through server's selected device capacity, not free
memory. On Apple Silicon (MPS) set `seeThrough.vramGiB` in `run.json` to the
unified-memory capacity (default 64), because MPS reports no CUDA VRAM for automatic
detection. If capacity is unknown, obtain it; do not guess. Process one pose at a
time without interrupting
other jobs. Qwen-Image-2.1 runs on its own instance with no offload switch; never
assume CUDA for it.

Match native layers and geometry to the source. Review eye, iris, lash and mouth
masks for this character; do not assume eye colour, skin tone or fixed coordinates.
Before local joint motion, inspect each isolated moving layer for baked-in shoulder,
torso or hair, and split or remake it before rigging
([motion-authoring.md](references/motion-authoring.md)).

Default to visibly expressive, pose-specific motion at desktop size within herdr's
native motion constraints: preparation, action, small overshoot and settling;
staggered head/body/limbs. Increase useful movement through rigging and timing, not
a global amplitude multiplier. Respect a request for restrained motion.

## Validation and delivery

Validate the staged pack with the native executable as described in
[character-creator](../character-creator/SKILL.md) (`tools/character-pack.py
validate`, native `pack preview --pose <id>` for each of the ten bindings, and
`tools/character-pack.py review`), then run `review-native.py` into a new directory
and inspect every capture yourself.
Check both eyes, low blink values (0.1–0.3), low mouth-open values (0.2–0.3),
intermediate smiles, head/gaze states, every phase and reaction, hit regions,
seams and transparency on light and dark backgrounds. Parser success, a valid
catalog or a generated contact sheet is not artistic acceptance.

Stage packs outside the repository's assets. Never overwrite a staged pack, review
directory or the bundled default except through `assemble-pack.py cutover`, which
validates natively and keeps the original. Report the pack, the native review
evidence, unresolved visual limits and the actual scope of rights checks. Treat
production as supervised and iterative, not guaranteed unattended completion. Keep
historical candidates and private evidence out of the final pack and report run disk
usage.

`assemble-pack.py stage` takes the v5 entry template, motion source and the
license, owner and attribution records from `tools.motionReference`, and refuses a
reference whose manifest `name` differs from `character.label`, because rights
records belong to one character. For a new character, create a private rig working
copy named after it (`tools/character-pack.py template --mode rig --id <id> --name
"<label>" --output <dir>`, see character-creator), replace its license, attribution
and owner records with this character's verified terms, regenerate its inventory,
and point `tools.motionReference` at it. Never ship another character's
authorization.
