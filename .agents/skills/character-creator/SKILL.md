# Herdr character creator

Use this skill to author a local v4 or v5 character pack and hand it to Herdr's native
validator. A pack is data, not an executable plugin. The creator tool never
runs code from a pack, never fetches a URL, and never treats a parser-only check
as full validation.

## Where the tool lives

From a source checkout, use:

```sh
python3 tools/character-pack.py --help
```

In a bundled application, use
`Contents/Resources/creator/character-pack.py`. The bundled creator uses its
sibling `Contents/Resources/default` (the bundled ten-pose Rubelia rig template).
For PNG authoring in a packaged app, supply an external PNG template with
`--templates-root PATH`. In a source checkout the rig creator uses
`assets/rubelia-default`. Optional Aurora and Coding Cat templates live in the
separate [herdr-characters repository](https://github.com/hanbong5938/herdr-characters).
The explicit
template root may be a parent containing `rig-default`, `png-example`, or
`default`, or the template directory itself; its render mode must match.

The native executable is required for acceptance. Resolution order is an
explicit `--native PATH`, `HERDR_DESKTOP_PET_BIN`, then a small set of source
and bundle paths including `Contents/MacOS/herdr-desktop-pet` and conventional
`target/{debug,release}` locations. If it cannot be found, the tool fails
clearly. It does not fall back to Python parsing, an image library, a browser,
or a fake renderer.

## Canonical workflow

The creator has one command for each authoring stage; command names are not
aliases:

* `init` is `template` (create a private `png` or `rig` working copy);
* `check` is `inventory --check` followed by native `validate`;
* `rig` assembles an authored rig entry without changing artwork;
* `capture`/`scrub` is the fixed-time native `preview` command;
* `review` captures the native phase/reaction review set;
* `sharing` is native-validated `package` or read-only `export`.

The executable is the production renderer and validator boundary. The Python
tool only applies bounded filesystem/inventory rules and invokes native
commands; it never executes pack payloads or substitutes a parser, browser,
image library, or fake renderer.

## Start from a template

Create a private working copy. The destination must not already exist:

```sh
HERDR_CHARACTERS_DIR=/absolute/path/to/herdr-characters
python3 tools/character-pack.py template \
  --mode rig \
  --id rubelia-work \
  --name "Rubelia Work Copy" \
  --output work/rubelia-work

python3 tools/character-pack.py template \
  --mode png \
  --templates-root "$HERDR_CHARACTERS_DIR/packs/png-example" \
  --id coding-cat-work \
  --name "Coding Cat Work Copy" \
  --output work/coding-cat-work
```

For a bundled rig creator, run this from the directory containing `Contents`:

```sh
python3 Contents/Resources/creator/character-pack.py template \
  --mode rig --id rubelia-work --output work/rubelia-work
```

You may instead pass `--templates-root Contents/Resources/default` explicitly.
The bundled v5 Rubelia template retains ten independent layered PSDs, complete
motion, and provenance. Copying a template does not grant a new artwork license.
PNG authoring requires a separate PNG template, such as
`herdr-characters/packs/png-example`; it is not shipped in this source repository.

Template IDs are ordinary shareable IDs. Do not use the reserved builtin
identity `default`; it is a store reference, not an authoring ID. The template
command copies only flat regular files and regenerates payload inventory hashes.

## Pack contract

Every v4/v5 root contains exactly one `manifest.json` plus its listed root-level
payload files. Filenames are printable ASCII, one path component, and are
case-collision free. Do not add directories, dotfiles, symlinks, hard links,
archives inside the pack, scripts, HTML, shaders, plugins, credentials, or
network configuration. The creator rejects special files, links, and executable
permission bits, and has bounded reads.

The manifest has `format: "herdr.character"`, `version: 4` or `5`, an internal
`render_mode` (v4: `png` or `rig`; v5: `rig`), dimensions, author/source/rights
metadata, entry/runtime declarations, and an exhaustive payload inventory. Optional
fields are omitted rather than written as `null`. Keep the complete license
text and provenance as payloads. A procedural source normally uses
`source.method: "procedural"`; hand-authored independent layers use
`"hand-layered"`; a user-supplied generated image may use
`"image-generated"` only with truthful provenance.

Finite authoring limits are deliberate: manifest 64 KiB; at most 128 payload
files; 64 MiB total expanded data including the manifest; ordinary payload 4 MiB;
entry and motion JSON 64 KiB; overrides JSON 256 KiB; 1,024px maximum dimension.
PNG retains its 1,000,000 decoded-pixel limit; rig permits 1,024 × 1,024 pixels.
PNG v4 has at most 32 unique frames, 112 references,
16 frames per phase, 12 per reaction, and 1..30 FPS. Rig motion has four phases
and four reactions, durations 1..30,000 ms, transitions no longer than 2,000
ms or the motion duration, no more than 40 tracks per motion, 32 keys per
track, 256 keys per motion, and 1,024 keys total. Keep native/PSD limits in
`native/rig/limits.json` authoritative; do not widen or duplicate them.

## Layered rig authoring

A rig is genuine source separation, not a flattened PNG or a PSD containing a
single baked screenshot. Include an independently authored neutral PSD and,
when used, an independently authored alternate PSD. The alternate must contain
changed source geometry and the same canvas dimensions; copying one flattened
image, recoloring it, or swapping a rendered frame is not a pose.

The canonical rig payloads are normally:

* `model.psd`: v4 neutral/base model; dimensions match the manifest (640x896 for optional Rubelia wardrobe packs, 384x512 for Aurora);
* `pose.psd`: independent alternate model, same dimensions;
* `rig-overrides.json`: layer order/depth/group, anchors, hidden underpaint,
  expressions, and interaction areas;
* `motion.json`: the four authored phase and four authored reaction tracks;
* `rig-entry.json`: base/alternate IDs and payload references;
* license, attribution, and source/provenance text payloads.

Use semantic layer names that match the override and native decoder contract.
Keep visible semantic parts independent (head/face, neck, torso/body, each
wing where applicable, each antenna, eye whites/irides/closed eyes, and mouth
shapes). Complete hidden underpaint can preserve movable pixels behind visible
layers, but it must be explicitly hidden and declared; it is not a substitute
for separation. Do not add unsupported pose-local or partial selectors.

Author explicit anchors for face/eyes/mouth/body pivot and explicit `head` and
`torso` interaction rectangles inside the canvas. Native validation checks
actual PSD records, dimensions, layer graph, anchors, and mesh/deformation
inputs, then prepares the production eye-stencil renderer and a bounded neutral
frame. Independent movement and artistic correctness still require rendered
review; a PSD header saying 384x512 is not sufficient.

`motion.json` tracks use actual native `RigParameters` names such as `angleX`,
`angleY`, `angleZ`, `eyeOpenL`, `eyeOpenR`, `eyeX`, `eyeY`, `irisScale`,
`mouthOpen`, `mouthForm`, `mouthCY`, `body`, `bust`, `bustY`, `eyeCAng`, and
`mouthScale`. Keys start at zero, are strictly increasing, finite, and remain
inside the native parameter range; values that would be clamped are rejected.
Use a phase's duration as its loop period. Reaction age is clamped by the
native effect duration, and an expired reaction is absent rather than restarted.
Do not invent `poseMix` tracks; pose selection is the entry's base/alternate
IDs and native transition handling.

## Assemble an authored rig entry

Use `rig` only in an existing private `render_mode: "rig"` directory. It
rewrites the existing entry payload after checking the explicitly supplied
root-local PSD/JSON files; it does not copy or alter artwork:

```sh
python3 tools/character-pack.py rig \
  --native Contents/MacOS/herdr-desktop-pet \
  --path work/aurora-moth-work \
  --base-file model.psd --base-id aurora-baseline \
  --pose-file pose.psd --pose-id aurora-alternate \
  --overrides-file rig-overrides.json --motion-file motion.json
```

`--pose-file` and `--pose-id` are an all-or-nothing pair. Without them the
entry has no alternate pose. Every filename is a single root-local component,
and every artistic ID is supplied by the author; the command never infers
pivots, edits PSD pixels, or rewrites artistic motion intent. The authored
`motion.json` phase and reaction `pose` values must match the IDs supplied to
`rig`; native validation rejects inconsistent references. The creator never
rewrites those artistic references or invents a replacement ID. Existing
`author`, `source`, rights/licenses, attributions, `persona`, and `dialogue`
metadata remain in `manifest.json`; inventory hashes are regenerated after the
entry write.

Native `pack validate` is mandatory before `rig` reports success. If authored
source is invalid, the edited private directory remains available for
correction, is not registered or imported, and must not be treated as an
approval. Supply truthful rights, attribution, and complete license/provenance
payloads before validation; the creator never infers permission from filenames,
metadata, or a public URL.

## Independent ten-pose rigs (v5)

Use v5 for independently illustrated whole-body poses, not a shared face pasted
onto several bodies. Each pose owns its PSD, complete overrides, eye/mouth paint,
anchors, interaction regions, and motion. Source-local expression extraction is
allowed; replacing the complete face with another pose's face is not.

Finalize optional empty plates before generating layer-order, cleanup, and other
layer references; record every omitted plate in provenance. Do not leave references
to a raster the exporter skips, and do not silently omit required face/eye/mouth
features. A successful authoring adapter is not proof that PSD clipping preserves
the visible iris: run the actual native loader and inspect the rendered eyes.
Preserve explicitly authored left/right eye identities. A turned face can place
both visible eyes on one side of its skin centroid; that is not grounds to merge
them or substitute a generic iris. Check neutral lip extraction against the source
at magnified scale: a nearby jaw or hair edge is not a mouth landmark.
Keep each own open/smile raster and its contour reference registered to the actual
neutral lip. Static composites can look aligned while native interpolation drifts:
inspect low-open values (0.2–0.3), intermediate smiles, and the full expression range
in native captures. A genuinely closed neutral contour has zero aperture; avoid
oversized smile apertures that turn the source paint into a grey smear.

Compare every pose's original master, neutral layer composite, and actual native
face at matched scale, then inspect the whole catalog together. Technical pack
validation alone does not establish artistic identity. Trace the source's inner
eye aperture rather than enlarging it with a coarse ellipse; genuinely narrow
or hooded eyes can have very few visible aperture or sclera pixels. Preserve
black/neutral lashes as well as brown lashes, and retain source iris paint across
the entire visible aperture instead of exposing stretched reconstruction pixels.
Closed-eye masks must exclude eyebrows and hair; inspect low-blink values
(0.1–0.3) for detached fragments, hooked curves, and disappearing lash paint.
Persist accepted contours and source-space mouth geometry in the authoring plan,
not only in an exported PSD or a one-off repair script.

Measure the capture window's actual backing scale before claiming pixel sizes;
do not assume a Retina 2× drawable. For live pointer verification, confirm that
the pet is the input recipient before posting events. Accessibility permission
does not establish delivery through a macOS lock-screen overlay; never bypass
that overlay or report unobserved clicks as successful reactions.

The manifest keeps the provenance, rights, and inventory envelope above. Set its
runtime to `herdr-native-rig` version `2`, with all three capabilities:
`mesh-deformation`, `eye-stencil`, and `independent-models`. Rig canvases support
up to 1024×1024; the separate PNG decoded-pixel limit does not apply to rig PSDs.
Its rig entry has exactly `version`, `initial`, `models`, and `bindings`.
Entry `version` is `2`; `initial` is `"waiting"`. There are 10..16 models, each
with exactly `id`, `file`, `overrides`, and `motion`. All filenames are root-local,
and a PSD/overrides/motion payload cannot be shared between model records.

Bindings name all ten semantic poses exactly: `waiting`, `writing`, `failed`,
`cancelled`, `disconnected`, `bored`, `happy`, `head-tap`, `torso-tap`, and
`head-pet`. Every binding references a different declared model. Each model's
motion uses its own artistic ID; all model canvases match the manifest.

Convert a selected immutable DAEMONLET source round with an explicit per-model
motion profile; the adapter never assigns a gesture to a native slot by itself:

```json
{
  "version": 1,
  "pose": "writing",
  "phases": {
    "idle": {"model": "outputs/characters/run/poses/writing/N9/model.json", "transition_ms": 120},
    "running": {"model": "outputs/characters/run/poses/writing/N9/model.json", "transition_ms": 120},
    "waiting": {"model": "outputs/characters/run/poses/writing/N9/model.json", "transition_ms": 120},
    "unknown": {"model": "outputs/characters/run/poses/writing/N9/model.json", "transition_ms": 120}
  },
  "reactions": {
    "head_tap": {"model": "outputs/characters/run/poses/writing/head-tap/model.json", "transition_ms": 90},
    "body_tap": {"model": "outputs/characters/run/poses/writing/body-tap/model.json", "transition_ms": 90},
    "pet": {"model": "outputs/characters/run/poses/writing/pet/model.json", "transition_ms": 120},
    "completion_observed": {"model": "outputs/characters/run/poses/writing/completion/model.json", "transition_ms": 120}
  }
}
```

Every `model` is either `selected` or an upstream-root-relative `model.json`;
its referenced `pose.json` must contain authored motion with the same artistic
pose ID. Reusing one source is allowed only when the profile says so in every
slot. Keyframe interpolation and `loop`/`once` playback are retained, source
pixel translations are scaled to the 1024px export, and exact decoded layer
names, normalized origins, and taper influence are retained. Selector-only or
ambiguous layer names fail. A local transform without an explicit source origin
also fails: upstream's omitted-origin raw layer center is not equivalent to an
explicit normalized mesh point after deformation. Omitted influence is converted
to the exact full-layer-weight native taper. Constant tracks become one exact
key. Sine tracks and envelopes fail because native v2 cannot represent them
exactly.

```sh
python3 tools/adapt-rig-source.py \
  --upstream-root /path/to/DAEMONLET \
  --model outputs/characters/run/poses/writing/N9 \
  --output /path/to/DAEMONLET/outputs/characters/run/native-writing \
  --size1024 --motion-profile work/writing-motion-profile.json
```

The output includes `model.psd`, `rig-overrides.json`, `motion.json`, and
`provenance.json`. Use `--raster-only` instead of `--motion-profile` only for an
explicit geometry/raster export; its result and provenance report
`completeMotionAdaptation: false` and must not be treated as a finished model.

Assemble an already authored v5 manifest and complete entry without changing art:

```sh
python3 tools/character-pack.py rig \
  --native native/target/release/herdr-desktop-pet \
  --path work/rubelia-ten-poses --models-file rig-entry.json
```

The native host prepares all bound CPU scenes before reporting catalog readiness,
using at most four decoder workers, the 30-second decode deadline, and the
`catalogDecodedBytes` aggregate RGBA quota in `native/rig/limits.json` (64 MiB).
Model entry uploads an already prepared scene rather than re-decoding its PSD.
Motion restarts on model entry. Legacy v4 base/alternate behavior is unchanged.

Review each semantic binding explicitly through the production executable:

```sh
native/target/release/herdr-desktop-pet pack preview \
  --path work/rubelia-ten-poses --pose head-tap \
  --phase idle --time-ms 140 --reaction head_tap --reaction-age-ms 140 \
  --output reviews/head-tap-140.png
```

`--pose` is a native preview option for v5 only. Repeat for every binding and
review expression ranges, timed clips, body continuity, and native hit regions.
The creator's four-phase/four-reaction `review` does not cover all ten bindings.
A valid catalog or a synthetic switching fixture is not artistic acceptance.

### Authoritative OMP failure and cancellation

Load `integrations/omp/herdr-pet-outcomes.ts` alongside Herdr's managed OMP state
extension; do not replace or modify the managed extension. For a private session
started inside Herdr:

```sh
omp --extension "$PWD/integrations/omp/herdr-pet-outcomes.ts"
```
Use an OMP version with the `agent_end.willContinue` contract; the real TUI
producer/consumer flow was verified with OMP 18.2.10. Earlier versions without
that continuation signal are not supported by this integration.


The companion only publishes for root interactive TUI contexts. It binds reports
to the actual OMP session identity and emits terminal success, failure, or
cancellation from OMP lifecycle/assistant stop reasons. `agent_end.willContinue`
keeps the turn running; tool output and a generic idle/done status are not proof
of failure or cancellation.

Native tracking rejects stale, duplicate, mismatched-session, and old-generation
reports. Startup/reconnect terminal metadata is a baseline, not a new reaction.
A fresh short turn can still be accepted when polling missed its running report.
New panes are discovered on the five-second reconciliation cycle, so this
metadata bridge does not guarantee subsecond delivery for newly created panes.
The companion is executable integration code outside the character pack; never
place it in the pack's data payloads.

## PNG authoring

A PNG v4 entry must contain all four phase clips (`idle`, `running`, `waiting`,
`unknown`) and all four reaction clips (`head_tap`, `body_tap`, `pet`,
`completion_observed`). Every unique frame named by either map must have one
explicit `regions` record with `head` and `body` rectangles entirely inside
the declared canvas. Alpha remains the final hit-test gate; regions are not a
license to claim transparent pixels are interactive.

Coding Cat's deterministic, standard-library-only drawing source lives in
`herdr-characters`. Run the following from that repository:

```sh
python3 tools/generate-character-examples.py --output work/coding-cat --replace
```
The example generator imports the existing original drawing utility, writes
four phase and four reaction frame payloads, and writes the complete v4 entry
and provenance metadata. Use a new output directory instead of `--replace`
when preserving an existing work tree.

Use those original drawing utilities or equivalent authored local geometry for
new frames; do not modify `sources/legacy-png`, which remains the legacy v1
compatibility fixture in the character repository. Copy or author only the resulting transparent PNG data
into a v4 pack, then author entry timing, reaction arrays, and every frame's
regions. Do not claim a copied legacy frame is an independent pose unless its
pixels really differ.

For every rig or PNG submitted for review, the author must supply the rights
status and the complete license/attribution/source records in the pack. Native
validation checks the resulting data contract; it does not grant rights or
approve the artwork.

## Provenance and image-generation path

A user may supply an image-generation result, but this is an optional authoring
path, not a runtime dependency. Before including it, record the actual provider
(if any), model/version (if any), prompt or source description needed to
reproduce provenance, source URLs or local source reference, and the user's
permission/license for both the pixels and any edits. Confirm that the provider
terms permit redistribution in a Herdr pack. Never infer a license from a
model name, file metadata, or a public URL; do not fill in a provider/model
claim that was not observed. If rights are unknown, stop and do not publish.
Do not make human-trial, human-authorship, or third-party claims without
records. Put the honest rights notice and complete license text in payloads;
`source.method: "image-generated"` alone grants no rights.

### Legacy v4 Rubelia wardrobe authoring (character repository)

The bundled default is the ten-pose Rubelia v5 rig in
`assets/rubelia-default`. It is not part of the optional character catalog.
Optional wardrobe packs, Aurora, Coding Cat, their source layers, and their
character-specific generators live in `herdr-characters`. See that repository's
README for gallery review and authoring instructions.

From the character repository, serve the checkout locally and open
`tools/rubelia-author.html`. Its approved identity master and source record are
in `sources/rig/rubelia/`. Preserve the source disclosures and terms.
The editor does not invoke an image-generation provider.

To assemble the existing lounge layers into a new unpacked pack:

```sh
export HERDR_DESKTOP_PET_SOURCE=/absolute/path/to/herdr-desktop-pet
node tools/assemble-rubelia.mjs \
  --source sources/rig/rubelia/lounge \
  --output work/rubelia-lounge \
  --outfit lounge \
  --reference-record sources/rig/rubelia/reference-decision.json
```

Keep `SOURCE_TERMS.txt` with source layers. Assembly writes a layered PSD and
v4 manifest, not a flattened screenshot. Use this application's native
`validate`, `review`, and `package` commands before importing or distributing.

## Inventory, native verification, and preview

After editing bytes, regenerate hashes. This command is local and mutates only
that manifest's payload inventory:

```sh
python3 tools/character-pack.py inventory --path work/aurora-moth-work
python3 tools/character-pack.py inventory --check --path work/aurora-moth-work
```

`--check` reports drift without rewriting. Full validation must invoke the
native executable, for either an unpacked directory or a `.herdrchar` archive:

```sh
python3 tools/character-pack.py validate \
  --native Contents/MacOS/herdr-desktop-pet \
  --path work/aurora-moth-work
```

Preview is also native and fixed-time. It prepares the complete backend before
capturing the PNG; it is not a parser or a screenshot of a source PNG:

```sh
python3 tools/character-pack.py preview \
  --native Contents/MacOS/herdr-desktop-pet \
  --path work/aurora-moth-work \
  --output previews/aurora-waiting-hit-overlay.png \
  --phase waiting --time-ms 1200 \
  --reaction head_tap --reaction-age-ms 240 \
  --hit-overlay
```
`--phase`, `--time-ms`, `--reaction`, and `--reaction-age-ms` are optional, but
ages are finite non-negative milliseconds. Preview outputs are no-overwrite.
Use a distinct output filename for each capture. The native preview freezes
its viewport at source dimensions with backing scale 1 and evaluates the
provided phase/effect ages; expired effects are omitted.

`--hit-overlay` asks the same frozen production render to sample and draw the
native semantic interaction regions (for example, head and torso) over the
image. It is diagnostic sampled geometry, not a replacement hit test, artwork
edit, parser result, or artistic approval. The overlay preserves the source
dimensions and alpha; baseline and overlay captures must be reviewed together.

`review` first validates the source natively, then captures all four phases
and all four reactions at one supplied fixed `--time-ms`. Each scene is written
twice (baseline and `--hit-overlay`) for 16 PNGs total; reaction scenes also
use the supplied `--reaction-age-ms` (default 0). The output directory must be
new and private, and is never overwritten. `review.json` records every actual
native command, controls, output byte count/SHA-256, and native JSON result:

```sh
python3 tools/character-pack.py review \
  --native Contents/MacOS/herdr-desktop-pet \
  --path work/aurora-moth-work \
  --output reviews/aurora-moth-1200 \
  --time-ms 1200 --reaction-age-ms 240
```

This is production-renderer evidence for author review, not a future report,
mock event sequence, or automatic artistic approval. No Herdr lifecycle event
is synthesized and no registry/store state is changed.

## Deterministic package and store export

Package only after the directory passes native validation. The creator writes a
stored ZIP32 `.herdrchar` with stable entry order, zero timestamps/extras,
untouched manifest/payload bytes, no overwrite, and a private temporary file.
It invokes native `pack validate` on that exact temporary archive before
publishing it at the requested path:

```sh
python3 tools/character-pack.py package \
  --native Contents/MacOS/herdr-desktop-pet \
  --path work/aurora-moth-work \
  --output dist/aurora-moth-work.herdrchar

python3 tools/character-pack.py validate \
  --native Contents/MacOS/herdr-desktop-pet \
  --path dist/aurora-moth-work.herdrchar
```

The native store export is read-only and preserves the selected revision's
manifest and payload bytes. It never changes selected/active state and refuses
to overwrite an existing destination:

```sh
python3 tools/character-pack.py export aurora-moth-work \
  --revision 0 \
  --output dist/aurora-moth-export.herdrchar \
  --native Contents/MacOS/herdr-desktop-pet
```

The native executable's `pack import --path PATH` remains the supported import
route. Import the unpacked directory or archive locally, then list/select it
through the normal native store flow. The creator does not execute anything
inside the pack and does not offer a renderer-mode selector to the user;
validated manifest content chooses the backend.

## Shipped examples and exact source commands

Optional templates and source artwork are maintained in `herdr-characters`;
only the bundled Rubelia default and its menu thumbnail stay in this application
repository. Set the character checkout path and use the generic creator here:

```sh
HERDR_CHARACTERS_DIR=/absolute/path/to/herdr-characters
python3 tools/character-pack.py inventory --check --path "$HERDR_CHARACTERS_DIR/packs/rig-default"
python3 tools/character-pack.py inventory --check --path "$HERDR_CHARACTERS_DIR/packs/png-example"
python3 tools/character-pack.py validate --native "$HERDR_DESKTOP_PET_BIN" --path "$HERDR_CHARACTERS_DIR/packs/rig-default"
python3 tools/character-pack.py validate --native "$HERDR_DESKTOP_PET_BIN" --path "$HERDR_CHARACTERS_DIR/packs/png-example"
```

Run those commands only with local source and the native executable available.
Do not treat a successful generator invocation, JSON parse, or archive write as
validation until native `pack validate` and (for a rig) native fixed-time
`pack preview` complete.
