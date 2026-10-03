# Upstream provenance

This skill is a fork of the DAEMONLET `create-pet-character` skill.

- Repository: https://github.com/ddol2ya/DAEMONLET
- Commit: `1f5b1560162e4370368b0ab55c32c3271e5921c0`
- Path: `skills/create-pet-character`
- Runtime used as a library (not vendored): DAEMONLET creator runtime
  `scripts/characters/` (`build-source-models.py`, `lib/*`, `source_eye_blinks.py`,
  `export-model-psd.mjs`) and `scripts/run-seethrough.mjs`,
  `scripts/seethrough-workflow.mjs`, `scripts/seethrough-profile.mjs`, referenced
  through `run.json` `tools.runtime`.

Line references in this skill's documents point to that commit.

## License

The upstream skill is released under the MIT License, Copyright (c) 2026 Momo
Motion Lab contributors. The full notice is retained unchanged in
[LICENSE.txt](LICENSE.txt) and applies to the upstream-derived portions of this
fork. Model weights, generated artwork and character rights are not covered by it;
see `external-dependencies.json` for recorded model terms.

## Purpose of the fork

1. Every image generation/edit step uses a private local ComfyUI Qwen-Image-2.1
   server (port 8189) instead of a hosted built-in image tool.
2. The deliverable is a herdr v5 ten-pose rig pack validated by this repository's
   native executable, not a Daemonlet archive.
3. The proven local pipeline that produced the bundled Rubelia default is encoded as
   config-driven scripts (`run.json`), with the fixes below.

## Files

Kept unchanged:

- `LICENSE.txt`
- `scripts/prepare-reference.py`
- `workflows/seethrough-api.json`

Modified:

- `SKILL.md` — rewritten for the local Qwen + See-through → herdr v5 pipeline;
  keeps reference suitability, framing choice, pilot-before-expansion, per-pose own
  artwork, dependency setup with approval and staged visual review.
- `references/production.md` — run layout, stage commands, Qwen edit procedure,
  registration, chroma, blink and completion rules.
- `references/visual-review.md` — eye/lid/mouth extraction, native inspection,
  misregistration rejection; review ledger replaced by native evidence records.
- `references/reference-preparation.md` — thigh-up large-head framing, isnet-anime
  matting.
- `references/motion-authoring.md` — herdr native motion constraints and the
  pipeline's motion derivation.
- `external-dependencies.json` — adds `qwenImage21`, `matting` and Apple Silicon
  profile entries.
- `scripts/check-environment.py` — read-only HTTP probes of the See-through and Qwen
  servers (`--seethrough-url`, `--qwen-url`); package checks only with
  `--comfy-root`; no NVIDIA-only assumption.

Removed (Daemonlet-only; herdr has no equivalent payload or flow):

- `agents/openai.yaml` (Codex agent metadata)
- `assets/persona.example.json`
- `references/persona.md`, `references/character-chat.md`, `references/updates.md`
  (persona, Character Chat metadata, update feed/publishing)
- `scripts/creator.mjs` (`info`, `check`, `decompose`, `build`, `finish`,
  `motion`, `capture`, `review`, `verify`, `payload`, `export`,
  `validate-persona`, `upgrade-persona`, `upgrade-chat`, `add-update-source`,
  `create-update-feed`, `publish-update-packs`)
- `scripts/license-status.mjs`
- Instructions for the built-in ImageGen / `image_gen` tool, `.petchar` export and
  app-UI import, click variants (`poseVariants`), `models.json`,
  `capture-source-models.mjs`, `build-independent-payload.mjs`,
  `verify-independent.mjs`, `export-pack.mjs` and the `production-review.json`
  ledger.

Added:

- `UPSTREAM.md` (this file)
- `workflows/qwen-image21-two-reference.json`
- `scripts/run_config.py`, `scripts/qwen_local.py`, `scripts/identity-correct.py`,
  `scripts/generate-sources.py`, `scripts/prepare-sources.py`,
  `scripts/build-rigs.py`, `scripts/expression_fixes.py`,
  `scripts/review-native.py`, `scripts/assemble-pack.py`

Herdr equivalents of removed commands: `scripts/assemble-pack.py`,
`scripts/review-native.py`, and this repository's `tools/character-pack.py`
(`validate`, `preview`, `review`, `package`) and native `pack import`
(see [character-creator](../character-creator/SKILL.md)).

## Fixes

| # | Upstream behaviour (commit `1f5b156`) | Fork rule | Documented in |
| --- | --- | --- | --- |
| 1 | `references/production.md:57-63`, `scripts/characters/finish-source-models.py:14,24-25,32-34`: 2048² green part atlas edited by a hosted built-in image tool, adopted wholesale. | Local Qwen-Image-2.1 two-reference edits on small crops (`TextEncodeQwenImage21` resolution 768, 25 steps, cfg 1, euler/simple, denoise 1); identity enters once in identity correction, composited only inside `faceEditRect` with outside-mask RGBA and alpha proven unchanged; poses inherit identity from the corrected authority; expressions use the pose's own head crop. | `references/production.md` |
| 2 | `scripts/characters/build-source-models.py:52-63` `register_panel`: translation-only ±12 px. | Similarity (scale + translation) registration with score/scale/shift limits that reject misregistered panels. | `references/production.md`, `references/visual-review.md` |
| 3 | `scripts/characters/lib/assemble_finish_art.py:5-44` `chroma_matte`: exact `#00FF00` key. | No green-sheet finishing; mask composite on originals; isnet-anime matting for subject alpha. | `references/reference-preparation.md`, `references/production.md` |
| 4 | `scripts/characters/build-source-models.py:68-110` `neutral_features` (`R−G>30` lashes, `mean>170` whites) and `scripts/characters/source_eye_blinks.py:4-9` `rebuild_lid` (warm-hue darkness). | See-through alpha contours (`sourceEyeMasks`, `sourceIrisMasks`), pigment check, hue-independent closed-lid extraction excluding front hair and eyebrows. | `references/visual-review.md` |
| 5 | `scripts/characters/build-source-models.py:143-148` `add_mouth_expressions`: fixed 1280-canvas pixel constants. | Constants scaled by measured face width; reviewed `neutralMouthMask` required. | `references/visual-review.md` |
| 6 | `references/reference-preparation.md:10`, `SKILL.md:21`: any supplied framing, full-body production allowed. | Thigh-up framing with an unusually large head. | `references/reference-preparation.md` |
| 7 | `scripts/characters/source_eye_blinks.py:29-45` `make_profile` via `scripts/characters/build-source-models.py:152` `blink_profiles`: per-pose measured lid profiles used raw, jittering at low blink values. | Gaussian regularization of aperture/closure trajectories (geometry only, no RGB/alpha edits), as recorded by the proven run. | `references/production.md` |
