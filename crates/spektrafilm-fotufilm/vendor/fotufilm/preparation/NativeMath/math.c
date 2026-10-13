#include <stdint.h>
// Preparation executes the same Float libm operations as the native source on this target.
#define HOST_UNARY(name) \
__attribute__((import_module("fotufilm_math"), import_name(#name))) float fotufilm_host_##name(float); \
float __wrap_##name(float x) { return fotufilm_host_##name(x); }
HOST_UNARY(expf)
HOST_UNARY(exp2f)
HOST_UNARY(logf)
HOST_UNARY(log1pf)
HOST_UNARY(log2f)
HOST_UNARY(log10f)
HOST_UNARY(tanf)
__attribute__((import_module("fotufilm_math"), import_name("powf"))) float fotufilm_host_powf(float, float);
float __wrap_powf(float x, float y) { return fotufilm_host_powf(x, y); }
// Keep the original CPU's Halide bank builder instead of WASI's unavailable-backend fallback.
__attribute__((import_module("fotufilm_native"), import_name("film_tile_build")))
int32_t fotufilm_host_film_tile_build(int32_t, int32_t, int32_t, const float *, int32_t,
    const float *, const float *, int32_t, uint32_t, int32_t, float *);
int32_t __wrap_fotufilm_film_tile_build(int32_t texels, int32_t supersample, int32_t markShape,
    const float *terms, int32_t termCount, const float *sublayers, const float *fractions,
    int32_t levels, uint32_t seed, int32_t record, float *light) {
    return fotufilm_host_film_tile_build(texels, supersample, markShape, terms, termCount,
        sublayers, fractions, levels, seed, record, light);
}
