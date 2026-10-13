#ifndef SPEKTRAFILM_FOTUFILM_BRIDGE_H
#define SPEKTRAFILM_FOTUFILM_BRIDGE_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct sf_fotufilm_invocation {
    const float *configuration;
    size_t configuration_count;
    const float *exposure;
    const float *film;
    const float *paper;
    size_t lut_float_count;
    int32_t lut_dimension;
    int32_t feature_mask;
    uint32_t seed;
    int32_t spatial_support;
} sf_fotufilm_invocation;
typedef struct sf_fotufilm_component {
    sf_fotufilm_invocation head;
    const float *stencils;
    size_t stencil_count;
} sf_fotufilm_component;
typedef int32_t (*sf_fotufilm_cancel)(void *context);
/* All arrays are caller-owned for this call. RGB inputs/outputs are planar, distinct,
   width*height floats each. No pointer is retained. tail!=NULL selects layered transport,
   including a valid zero-component (zero exposure) plan. 0 success, 1 cancellation,
   negative original/native error. Output is unpublished until success. Cold Halide JIT
   cannot be interrupted: cancellation is checked between bounded rendering calls. */
int32_t sf_fotufilm_process(
    const sf_fotufilm_invocation *base, const sf_fotufilm_invocation *tail,
    const sf_fotufilm_component *components, size_t component_count, int32_t record_channels,
    const float *tiles, size_t tile_float_count,
    const float *input_r, const float *input_g, const float *input_b,
    float *output_r, float *output_g, float *output_b,
    int32_t width, int32_t height, sf_fotufilm_cancel cancel, void *cancel_context,
    char *error, size_t error_capacity);
size_t sf_fotufilm_configuration_count(void);
size_t sf_fotufilm_lut_float_count(void);
size_t sf_fotufilm_tile_float_count(void);
const char *sf_fotufilm_source_revision(void);
/* Preparation uses the same target-native Float libm as the original Swift model. */
float sf_fotufilm_expf(float x);
float sf_fotufilm_exp2f(float x);
float sf_fotufilm_logf(float x);
float sf_fotufilm_log1pf(float x);
float sf_fotufilm_log2f(float x);
float sf_fotufilm_log10f(float x);
float sf_fotufilm_powf(float x, float y);
int32_t sf_fotufilm_film_tile_build(int32_t texels, int32_t supersample,
    int32_t mark_shape, const float *terms, int32_t term_count, const float *sublayers,
    const float *fractions, int32_t levels, uint32_t seed, int32_t record, float *light);
#ifdef __cplusplus
}
#endif
#endif
