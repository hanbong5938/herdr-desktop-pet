# Expressive motion from independent illustrations

Default to movement that is visible at the intended desktop size. A changing vertex
that moves less than a pixel on screen is not acting. Preserve the user's desired
character and motion intensity; a focused working pose can be calmer than a
celebratory reaction. Native pack rules are owned by
[character-creator](../../character-creator/SKILL.md); this file applies them to the
ten-pose pipeline. Upstream line references are to DAEMONLET commit
`1f5b1560162e4370368b0ab55c32c3271e5921c0` ([UPSTREAM.md](../UPSTREAM.md)).

## Herdr native motion constraints

Every pose model has one `motion.json` (native rig motion v2) and herdr validates it
natively; a clip that violates these rules fails the whole pack:

- **Eight slots, all required.** Four phases `idle`, `running`, `waiting`, `unknown`
  and four reactions `head_tap`, `body_tap`, `pet`, `completion_observed`. Each clip
  uses the model's own artistic pose ID, has `playback` (`loop`/`once`) and
  `layers`, a duration of 1–30000 ms and `transition_ms` at most
  `min(duration, 2000)`.
- **Limits.** At most 40 tracks per clip (parameter tracks plus layer transform
  tracks), 1–32 keys per track, at most 256 keys per clip and 1024 keys per motion
  file. Parameter values stay inside native bounds; layer translations stay within
  the canvas size, rotation within ±360°, scale within 0–4.
- **Keyframes only.** Native v2 represents keyframe interpolation exactly; sine
  tracks and envelopes cannot be represented and fail the adapter. Constant tracks
  become one key. Author oscillation as explicit keys.
- **Explicit origins.** Every local layer transform carries an explicit normalized
  `origin` (0–1 in the layer's mesh) and an `influence` (`axisX`, `axisY`,
  `start`, `end`; start < end within −2…2). An omitted origin is not equivalent to
  the raw layer centre after deformation and fails. Layer names are the exact
  decoded runtime names; selector-only or ambiguous names fail.
- **Semantic slots.** Herdr selects clips by phase and reaction, not by pose name:
  for example `writing` is shown for `running`, the reactions drive `head-tap`,
  `torso-tap`, `head-pet` and `happy` (`completion_observed`). Every slot of every
  model still needs a complete, sensible clip because the host can enter any slot on
  any model.

These replace upstream `skills/create-pet-character/references/motion-authoring.md:20-34`
(the Daemonlet motion-plan command with `gesture` timing presets and its capture
gate) and the click-variant planning of line 7.

## How the pipeline authors motion

`build-rigs.py build` writes each `native-models/<pose>/motion.json` from the
`tools.motionReference` pack's matching `<pose>-motion.json` (a complete, natively
valid eight-slot profile):

- phase/reaction timing, playback and dimensionless expression tracks are kept;
- global head/body amplitudes (`angleX`, `angleY`, `angleZ`, `body`, `bust`,
  `bustY`) are scaled down for the newly measured face;
- the reference's local layer transforms are dropped, because their pivots belong to
  another body; local motion is re-authored on this pose's own measured hand layers
  with explicit origins from the hand bounds and keyframed translation for the tap
  and pet reactions (and idle/waiting gestures where the pose calls for it);
- the result is checked against the native track/key limits before it is written.

This is a conservative starting point, not finished acting. Review it natively and,
when a pose needs a different action, author a new round with edited motion that
still satisfies every constraint above.

## Plan the action before rigging

For each pose record a distinct intent, the moving body parts, fixed contacts and the
main action beat. Examples: weight shift while waiting; downward focus and a small
thinking nod while working; recoil and recovery on failure; acknowledging gesture on
cancellation; gaze followed by head/body turn when disconnected; fidget during
boredom; lift and settle on happiness; leaning into a petting gesture. Herdr has one
model per pose and no click variants, so each reaction must read clearly on its own.

Use a small preparation in the opposite direction, a clear main action, a restrained
overshoot and a settling tail, all as explicit keys. Let gaze lead the head,
head/shoulders lead the body or arm, and hair/ribbons follow with a delay. Avoid
moving every part with the same curve. Loops must join continuously; a once-only
reaction settles and holds instead of snapping back. Entry/exit, rapid repeated
taps, task preemption and re-entry need review.

## Source-specific rig

- **Inspect layer contents before authoring local movement.** A See-through label
  such as `handwear` is not proof that the layer contains only a movable arm.
  Compare each isolated layer with the composite. If shoulder/torso, hair, skin or
  another object that must move differently is baked into it, the layer is not
  ready: split it into independently controlled parts and reconstruct revealed
  paint, or re-decompose/remake that part, in a new round. Do not pass a mixed layer
  by reducing amplitude, hiding the seam behind hair or declaring one pivot pinned.
- Review a repaired split in neutral and over the motion range: no missing backing,
  duplicated paint/outline, transparent holes, order errors or stray fixed pixels.
  Pieces meeting at a joint need shared deformation near the attachment plus
  reviewed overlap/underpaint; a tapered influence suits an already connected part
  and does not fix mixed content.
- Measure shoulder/elbow/wrist and hair/accessory roots on the selected
  illustration. Use separate movable regions, explicit joint-local origins and
  tapered influence so a sleeve root stays attached while its cuff moves. Never
  rotate a whole painted sleeve around the canvas centre.
- A hand touching the cheek, hair, another hand or a prop follows the head
  (`headFollowLayers`, `frontHands` in geometry) and must not also receive an
  unrelated arm sway. Check the contact at motion peaks and during transitions.
- Plan source coverage for the intended motion. Large turns, newly exposed surfaces
  or crossed limbs may need reconstructed hidden paint or a different illustration.
  Do not stretch a flat source until the face or body distorts.
- Secondary hair/ribbon motion uses anchored roots and smaller delayed movement.
  Recheck transparent outlines and occlusion as locks move apart.

## Motion acceptance

Capture natively: `review-native.py` samples entry, middle, near-expiry and key
times of each pose's intended slot, and `pack preview --pose <id> --phase …
--time-ms … [--reaction … --reaction-age-ms …]` captures any other time. Inspect
desktop-size playback and enlarged action peaks on light and dark backgrounds.
Reaction captures must fall inside the host's effect lifetime; an expired effect is
omitted from the preview. Set a pose-appropriate travel target for at least one main
moving part; document intentional small movement rather than passing nearly static
output.

Check full shoulder/wrist seams, finger contact, neckline, overlapping clothes and
hair gaps at every sampled time, plus identity and eye/mouth review after any rig
change. Numeric travel can come from a detached limb and a few points can miss a
hole between them, so a passing measurement never replaces looking at the frames.
A code fix is not a repaired character until the new native candidate has passed
inspection.
