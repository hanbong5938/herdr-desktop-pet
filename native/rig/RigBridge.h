#ifndef HERDR_RIG_BRIDGE_H
#define HERDR_RIG_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void *HerdrRigHandle;
/* Native executable ABI identity, independent of character-pack versions. */
#define HERDR_RIG_ABI_VERSION 1U
typedef struct {
    uint32_t version;
    uint32_t asset_size;
    uint32_t model_size;
    uint32_t token_size;
    uint32_t intent_size;
    uint32_t hit_size;
    uint32_t anchor_size;
    uint32_t speech_anchor_snapshot_size;
} HerdrRigABIInfoV1;

typedef struct {
    const char *id;
    const char *file_path;
    const char *overrides_path;
    const char *motion_path;
} HerdrRigModelInput;

typedef struct {
    uint32_t width;
    uint32_t height;
    const char *base_path;
    const char *pose_path;
    const char *overrides_path;
    const char *base_pose_id;
    const char *pose_id;
    const HerdrRigModelInput *models;
    uint32_t model_count;
    const uint8_t *bindings;
    int32_t initial_pose_kind;
    /* Borrowed from the sealed Rust asset for the duration of herdr_rig_create.
     * The native host copies these bytes before returning. */
    const uint8_t *initial_motion_bytes;
    size_t initial_motion_length;
} HerdrRigAssetInput;

typedef struct {
    const char *operation_id;
    const char *reference_id;
    uint64_t reference_revision;
    const char *content_digest;
    uint64_t backend_epoch;
} HerdrRigTokenInput;

typedef struct {
    double now_seconds;
    double phase_age_seconds;
    double effect_age_seconds;
    double effect_duration_seconds;
    double pointer_x;
    double pointer_y;
    int32_t phase;
    int32_t effect;
    uint32_t visible;
    uint32_t frozen;
    int32_t pose_kind;
    double pose_age_seconds;
} HerdrRigIntent;

typedef struct {
    uint8_t alpha;
    uint8_t region; /* 0 = none, 1 = head, 2 = body; a known alpha sample may be zero */
    uint16_t reserved;
    double source_x; /* source coordinates are NaN when no semantic region was sampled */
    double source_y;
    uint64_t frame;
    uint64_t viewport_epoch;
    uint64_t input_epoch;
} HerdrRigHit;
/* Source-canvas coordinates, top-left origin; no GPU readback. */
typedef struct {
    double x0;
    double y0;
    double x1;
    double y1;
} HerdrRigAnchor;

/* A single coherent main-thread observation of speech geometry and its owner.
 * 0 = invalid, 1 = temporarily unavailable, 2 = ready without visible
 * geometry, 3 = ready with a source-canvas anchor. */
typedef struct {
    uint32_t status;
    uint32_t reserved;
    uint64_t backend_epoch;
    uint64_t input_epoch;
    uint64_t anchor_epoch;
    uint64_t viewport_epoch;
    uint32_t canvas_width;
    uint32_t canvas_height;
    double viewport_width;
    double viewport_height;
    double backing_scale;
    HerdrRigAnchor anchor;
} HerdrRigSpeechAnchorSnapshot;

/* Safe to call before passing any typed rig input or creating a host. */
void herdr_rig_abi_info_v1(HerdrRigABIInfoV1 *out_info);

/* Every error returned through an out_error is allocated by strdup and must be
 * released with herdr_rig_error_free.  A successful call always writes NULL. */
void herdr_rig_error_free(char *error);

int32_t herdr_rig_create(const HerdrRigAssetInput *asset,
                         const HerdrRigTokenInput *token,
                         HerdrRigHandle *out_handle,
                         char **out_error);
int32_t herdr_rig_poll(HerdrRigHandle handle, uint32_t cancel_requested,
                       uint32_t *out_state, char **out_error);
void herdr_rig_cancel(HerdrRigHandle handle);
void herdr_rig_destroy(HerdrRigHandle handle);

/* The view is returned retained.  The Rust adapter consumes that retain exactly
 * once with Retained::from_raw.  The candidate remains hidden until activate. */
int32_t herdr_rig_view(HerdrRigHandle handle, void **out_view, char **out_error);
int32_t herdr_rig_update(HerdrRigHandle handle, const HerdrRigIntent *intent,
                         char **out_error);
void herdr_rig_set_viewport_epoch(HerdrRigHandle handle, uint64_t viewport_epoch);
int32_t herdr_rig_prepare_surface(HerdrRigHandle handle, const HerdrRigIntent *intent,
                                  double width, double height, double backing_scale,
                                  uint64_t viewport_epoch, uint32_t *out_ready,
                                  char **out_error);
int32_t herdr_rig_activate(HerdrRigHandle handle, char **out_error);
void herdr_rig_set_visible(HerdrRigHandle handle, uint32_t visible);
/* Zero means unavailable.  Captures must retain and recheck this generation;
 * recovery and visibility changes invalidate it, pose motion does not. */
uint64_t herdr_rig_input_epoch(HerdrRigHandle handle);
/* Freshness gates new semantic actions without discarding a same-owner capture. */
uint32_t herdr_rig_input_ready(HerdrRigHandle handle);
int32_t herdr_rig_speech_anchor_snapshot(HerdrRigHandle handle,
                                         HerdrRigSpeechAnchorSnapshot *out_snapshot,
                                         char **out_error);
int32_t herdr_rig_hit(HerdrRigHandle handle, double x, double y,
                      HerdrRigHit *out_hit, uint32_t *out_has_hit, char **out_error);
/* Cached union across every reachable pose/model, normalized source-canvas
 * coordinates from the top-left. Zero means there is no visible envelope. */
uint32_t herdr_rig_display_bounds(HerdrRigHandle handle, HerdrRigAnchor *out_bounds);
/* hit_overlay requests the bounded semantic grid over the captured preview. */
int32_t herdr_rig_preview_png(HerdrRigHandle handle, const HerdrRigIntent *intent,
                              uint32_t hit_overlay, uint8_t **out_bytes, size_t *out_length,
                              char **out_error);
void herdr_rig_bytes_free(uint8_t *bytes, size_t length);
char *herdr_rig_last_error(HerdrRigHandle handle);

#ifdef __cplusplus
}
#endif

#endif /* HERDR_RIG_BRIDGE_H */
