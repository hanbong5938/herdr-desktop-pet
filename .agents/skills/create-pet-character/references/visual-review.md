# Visual review and candidate acceptance

Read for new artwork/rig production and visual repairs. Use this alongside
[production.md](production.md); it does not grant installation, download or
publishing permission. Upstream line references are to DAEMONLET commit
`1f5b1560162e4370368b0ab55c32c3271e5921c0` ([UPSTREAM.md](../UPSTREAM.md)).

## Pilot before expansion

Review `waiting`, `writing` and `head-tap` through source → matte → layers →
geometry → expression sheet → built round before generating the remaining poses.
A single overview or a successful decomposition does not establish the pipeline.
When a defect repeats across poses, stop expansion, identify the shared cause, repair
one pilot and verify it before batch application. Native visual and motion passes
happen on the staged ten-model pack, because staging and native review require all
ten models.

## Four separate pose checks

| Stage | Evidence required before recording pass |
| --- | --- |
| `source` | Identity, chosen framing, outfit/accessories and pose match the authority; matte alpha and enlarged outline previews on white/dark (`prepare-sources.py matte` review composites). |
| `layers` | Isolated See-through parts match their intended motion: no fixed shoulder/torso/hair baked into a moving limb. Split/reconstruct/remake contaminated parts before rigging. Face is the face rather than a larger skin island; hair contains no skin/clothing contamination; expression backing contains no old eyes/brows/lips. |
| `visual` | Native renderer only: both eyes, iris/pupil, lashes, brows, nose, mouth and surrounding skin; low blink, low mouth-open and intermediate smile values; head/gaze/combined states; seams and transparency on light and dark backgrounds. |
| `motion` | Pose-specific action visible at desktop size, continuous loop/settle, attachment boundaries and hand contact, delayed hair/accessory motion, silhouette and clothing boundaries throughout the clip including exact key peaks, every phase and reaction. |

Native evidence comes from `scripts/review-native.py --native BIN --path PACK
--output NEW_DIR` and the character-creator commands (`tools/character-pack.py
validate`, `preview`, `review`; native `pack preview --pose <id> --phase … --time-ms
…`). `review-native.py` captures each own-pose model (neutral, hit-region overlay,
entry/middle/near-expiry and low-blink/mouth/head-angle key times), every unforced
phase and reaction, and light/dark contact sheets. Open the PNGs themselves; a
contact sheet, `review.json` status or successful validation is not a visual pass.
Captures are frozen at backing scale 1; do not assume Retina pixel sizes.

## Eye, lid and mouth extraction

Fix 4 replaces upstream warm-brown colour heuristics:

- DAEMONLET `scripts/characters/build-source-models.py:68-110` (`neutral_features`):
  eye whites were sampled with `mean > 170`, and lash lines selected with
  `R − G > 30, G < 185` near the lash alpha.
- DAEMONLET `scripts/characters/source_eye_blinks.py:4-9` (`rebuild_lid`): closed-lid
  darkness scored as `(183 − G)/80 × (R − G − 18)/30`, i.e. warm hue only.

Known failures of those rules: **green irises** are dropped from the aperture,
**black or neutral lashes** vanish because they have no red excess, and **red hair**
or red-brown front locks are picked up as lash or lid line. Rules instead:

1. Apertures and irises come from the pose's own See-through alpha contours
   (`eyewhite-*`, `irides-*`), written by `build-rigs.py geometry` to
   `sourceEyeMasks` and `sourceIrisMasks`. The build refuses to fall back to colour
   inference when either mask is missing. Keep all visible iris alpha, including
   green paint and the dark rim; do not fit an ellipse or a circle.
2. A pigment check confirms the masks sit on painted source: the iris intersects the
   visible aperture, the aperture has real colour contrast, the lash contour lies on
   dark source pixels (dark by luminance or warm-dark, never hue alone) and the eye
   white overlaps painted source. A failed check means the decomposition or mask is
   wrong; correct it rather than loosening the check.
3. Closed-lid extraction (`expression_fixes.py`) is hue-independent: lash/lid ink is
   detected as luminance contrast below the local skin level inside the eye anchor
   box, and the pose's own front-hair and eyebrow alpha are excluded. It must not
   depend on warm hue.

Fix 5 replaces DAEMONLET `scripts/characters/build-source-models.py:143-148`
(`add_mouth_expressions`): fixed 1280-canvas constants (half width 30–43 px, search
rows ±18/19, open width 16–32 px, smile width 22–45 px) were tuned for neutral mouths
of about 21–31 px (`anchors['mouth']` x1−x0). The fork scales every constant by
k derived from the reviewed neutral mouth width, not the face width: k = 1 inside
21.4–30.7 px (the upstream behaviour), otherwise width ÷ the nearer range end. The
search window stays inside the selected own face component: its top below the
nose, its bottom at least 8·k px above the face bottom (never reaching the chin
contour), and its sides 4·k px inside the face's horizontal extent on every window
row. A feature that touches both window sides or is wider than 2× the neutral width
is a jaw, contour or shading component and stops the build with a `ValueError`;
treat it like a misregistered sheet. Geometry must supply the reviewed
`neutralMouthMask`. The neutral mouth must lie inside the face, below both eyes,
and contain the complete lip without jaw or hair edges.

Inspect natively, because static composites can align while interpolation drifts:

- **Low blink 0.1–0.3**: detached lash fragments, hooked curves, disappearing lash
  paint, exposed iris under a nearly closed lid, eyebrows or hair pulled into the
  lid. `review-native.py` captures a key in this range when the clip has one;
  otherwise add `pack preview` captures at the clip times where the blink passes
  0.1–0.3.
- **Mouth open 0.2–0.3** and intermediate smiles: the open/smile rasters must stay
  registered to the neutral lip; no grey smear, doubled lip or jump at the start of
  the transition. A genuinely closed neutral contour has zero aperture. Check
  `fixes.mouthExpressions.mouths.*.resize` in `own-source-record.json`. On the
  proven Rubelia sheets it was 1.1–1.8. A much larger value means only a fragment of
  a faint lip stroke was found and enlarged; the proven `writing` smile found 10 of
  about 22 px and was resized 3.5×. Inspect that sprite closely.
- Both eyes separately, at enlarged scale, on light and dark backgrounds.

## Misregistration

An expression panel whose Qwen redraw shifted or rescaled the face produces eyes and
mouths that slide off the neutral face during blink or speech. The build's
similarity registration (production.md, fix 2) rejects such panels instead of using
them. Both panels come from one Qwen render, so `expression_fixes.py` estimates
scale and translation on panel 1 (open eyes; nose, eye and eyebrow patch) and
registers panel 0 (closed eyes; eye interiors excluded) at panel 1's scale with a
translation search only. Panel 0's scale is then checked by matching panel 0
directly to panel 1 on their shared content (sheet crop inset 24 px, eyes and
mouth excluded with 16 px padding). A sheet is rejected when:

- panel 1 scale is outside 1 ± 0.10 (searched 0.82–1.18), its translation reaches
  the ±24 px search limit, or its normalized score exceeds 0.35;
- panel 0's translation reaches ±14 px from panel 1's, or its normalized score at
  panel 1's scale exceeds 0.55;
- the panel 0 → panel 1 scale is outside 1 ± 0.03.

The normalized score is the upstream metric divided by the score of a featureless
patch. 0 means identical structure; an uncorrelated panel scores about 2. On the
ten proven Rubelia sheets panel 1 measured scale 0.990–1.055 and score 0.104–0.174.
Panel 0 measured an offset of at most 10.5 px and score 0.059–0.371, and the
panel 0 → panel 1 scale was 0.996–1.005. Panel 0 rescaled alone by 8% gave
0.922–0.931. Treat a rejection as a failed attempt and regenerate the expression
sheet with a new attempt. Also reject on inspection, even when the score passed,
if:

- the closed-eye lid line or the open mouth sits visibly above/below the neutral
  feature at any intermediate value;
- the eyes appear to change size between neutral and expression;
- a feature shows a double contour (old and new paint) during the transition.

Record the rejection with the panel's recorded scale, translation and score.

## Diagnose before another repair

- Compare source, isolated part and native render at the same coordinates. Decide
  whether the error is painted in the source, belongs to a mask, comes from
  alpha/colour compositing, or appears only during deformation.
- A white crescent on an iris may come from antialiasing being blended twice. Do not
  hard-code an iris colour or replace eyes with circles. Preserve the source iris and
  measured lid contours, including eyes partly hidden by hair.
- Inspect skin underneath every movable eye, eyebrow and lip. Rectangular source
  patches can hide a ghost in neutral and expose it during transitions. Bounds are a
  search region, not a final rectangular alpha mask.
- `geometry.faceComponentPoint: [x,y]` selects the skin component containing a
  reviewed point in that pose. Decide excluded components' semantic class
  separately; never automatically call discarded skin legwear.
- `geometry.sourceEyeMasks`/`sourceIrisMasks` and `featureMasks` (per-part trim
  masks) do not create missing paint or repair a bad iris/lid; inspect the resulting
  geometry and backing.
- `geometry.neutralMouthMask` must include the complete source lip, not just its
  darkest connected piece; inspect every transition against the cleared skin.
- A noisy transparent outline usually originates in background remnants or matte
  fringes. Alpha erosion/blur alone can remove the intended line or thicken it.
  Rematte or repair the source outline with the original line colour/weight and
  inspect gaps around fingers, ribbons and hair. Never make pale clothing
  transparent.
- Use floating point for colour subtraction/blending, then clamp and quantize once.
- If a second candidate repeats the same defect or changes identity, stop blind
  retries. Reinspect decomposition/registration, choose a different repair or report
  the actual blocker.

## Recording review

There is no review ledger tool. Record each stage verdict (pose, stage, pass/fail,
notes, inspected evidence paths relative to RUN) in the run next to the evidence,
bound to the hashes the evidence itself records: `matting.json`, `generation.json`,
`own-source-record.json` for pose stages, and `review.json` (`source_pack_digest`
and per-capture SHA-256) for native stages. Any asset change makes earlier verdicts
stale; a later failure supersedes a pass; rejected candidates stay preserved.
Delivery additionally needs native `validate` of the exact final directory or
archive (character-creator), observed import/selection when the pack is
distributed, and a rights record that states unresolved terms instead of claiming
clearance. These replace upstream
`skills/create-pet-character/references/visual-review.md:35-63` (fingerprint ledger,
payload gate, archive binding).

Report material visual limits rather than “flawless”. Never hand off a rejected
candidate.

## Working files

Keep all sources/candidates, captures, model locations, environment logs and private
paths out of Git and distributable packs. Review status is separate from disk usage:
report large run size and offer a retention plan. Preserve original, selected and
rejected evidence; do not delete it automatically. When resuming older runs, prefer
actual artifacts and their provenance over free-form progress notes.
