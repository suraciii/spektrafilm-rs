#include <libraw/libraw.h>
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <memory>
#include <stdexcept>
#include <vector>

#if defined(SF_RAW_OPENMP)
// Declared without enabling OpenMP in this bridge: the selected LibRaw package
// supplies the runtime at link time, not the bridge compiler's default runtime.
extern "C" int omp_get_max_threads();
extern "C" void omp_set_num_threads(int);
#endif

namespace {
class OrderedXTrans {
#if defined(SF_RAW_OPENMP)
    const int previous_threads_;
#endif
public:
    explicit OrderedXTrans(unsigned filters)
#if defined(SF_RAW_OPENMP)
        : previous_threads_(filters == 9 ? omp_get_max_threads() : 0)
#endif
    {
#if defined(SF_RAW_OPENMP)
        // OpenMP's nthreads-var is local to the calling task. Do not change
        // environment variables or serialize independent decoder operations.
        if (previous_threads_) omp_set_num_threads(1);
#else
        (void)filters;
#endif
    }
    ~OrderedXTrans() noexcept {
#if defined(SF_RAW_OPENMP)
        if (previous_threads_) omp_set_num_threads(previous_threads_);
#endif
    }
    OrderedXTrans(const OrderedXTrans&) = delete;
    OrderedXTrans& operator=(const OrderedXTrans&) = delete;
};
}

// Reference decode through LibRaw's full CPU pipeline. This stays the CPU
// reference and the automatic route's fallback; it is never used by the
// device RAW path.
extern "C" int sf_raw_decode(const unsigned char *bytes, size_t length, int camera_wb,
                              float **pixels, unsigned *width, unsigned *height,
                              char *error, size_t error_length) {
    *pixels = nullptr;
    try {
        LibRaw raw;
        auto check = [](int code) {
            if (code != LIBRAW_SUCCESS) throw std::runtime_error(libraw_strerror(code));
        };
        check(raw.open_buffer(const_cast<unsigned char *>(bytes), length));
        if (raw.error_count() > 0) throw std::runtime_error("Data error or unsupported file format");
        check(raw.unpack());
        if (raw.error_count() > 0) throw std::runtime_error("Data error or unsupported file format");
        auto &p = raw.imgdata.params;
        p.output_color = 6; // ACES (AP0), rawpy.ColorSpace.ACES.
        p.output_bps = 16;
        p.no_auto_bright = 1;
        p.gamm[0] = 1.0;
        p.gamm[1] = 1.0;
        p.use_camera_wb = camera_wb;
        p.use_auto_wb = 0;
        p.user_qual = -1; // rawpy default: LibRaw chooses AHD (its native sensor defaults).
        p.adjust_maximum_thr = 0.75f; // rawpy.Params default, explicit across LibRaw versions.
        p.exp_correc = -1;
        p.exp_shift = 1.0f;
        p.exp_preser = 0.0f;
        p.bright = 1.0f;
        p.highlight = 0;
        p.user_flip = -1;
        p.user_black = -1;
        p.user_sat = -1;
        {
            const OrderedXTrans ordered(raw.imgdata.idata.filters);
            check(raw.dcraw_process());
        }
        if (raw.error_count() > 0) throw std::runtime_error("Data error or unsupported file format");
        int status = LIBRAW_SUCCESS;
        std::unique_ptr<libraw_processed_image_t, decltype(&LibRaw::dcraw_clear_mem)> image(
            raw.dcraw_make_mem_image(&status), LibRaw::dcraw_clear_mem);
        check(status);
        if (!image || image->type != LIBRAW_IMAGE_BITMAP || image->bits != 16 || image->colors != 3)
            throw std::runtime_error("LibRaw did not return a 16-bit RGB bitmap");
        const size_t count = static_cast<size_t>(image->width) * image->height * 3;
        float *out = static_cast<float *>(std::malloc(count * sizeof(float)));
        if (!out) throw std::bad_alloc();
        const auto *values = reinterpret_cast<const unsigned short *>(image->data);
        for (size_t i = 0; i < count; ++i) out[i] = static_cast<float>(values[i]) / 65535.0f;
        *pixels = out;
        *width = image->width;
        *height = image->height;
        return 0;
    } catch (const std::exception &e) {
        if (error_length) std::snprintf(error, error_length, "%s", e.what());
        return 1;
    } catch (...) {
        if (error_length) std::snprintf(error, error_length, "%s", "Unknown LibRaw failure");
        return 1;
    }
}
extern "C" void sf_raw_free(float *pixels) { std::free(pixels); }
extern "C" const char *sf_raw_version() { return LibRaw::version(); }

// ---------------------------------------------------------------------------
// Unpacked sensor exposure for the device RAW path.
//
// This entry point performs file parsing and compressed-sample unpacking only.
// Format-required reconstruction (for example float delta prediction and its
// decoder maximum) belongs to unpack. The separable CONVERTFLOAT_TO_INT option
// is explicitly disabled; normalization and black folding then run on-device.
// No dcraw_process call, demosaic, white balance, geometric correction or colour
// assignment occurs here. Integer black metadata folding is image-independent.
// ---------------------------------------------------------------------------

namespace {

/// Exact mirrors of LibRaw's private constants, used to reproduce
/// `convert_to_rgb()`'s camera→XYZ matrix and `cielab()`'s homogeneity
/// matrix. LibRaw keeps them in `LibRaw_constants`, whose symbols are not
/// part of the stable shared-library ABI.
constexpr double kXyzRgb[3][3] = {
    {0.4124564, 0.3575761, 0.1804375},
    {0.2126729, 0.7151522, 0.0721750},
    {0.0193339, 0.1191920, 0.9503041}};
constexpr float kD65White[3] = {0.95047f, 1.0f, 1.08883f};

/// The period lengths a LibRaw CFA tile can have. Every LibRaw pattern
/// (`filters` bit field: 4×2; the 16×16 `filters == 1` table; the 6×6
/// X-Trans table) divides one of these, and all divide 48.
constexpr int kCfaWindow = 48;
constexpr int kCfaPeriods[] = {1, 2, 3, 4, 6, 8, 12, 16, 24, 48};

void copy_message(char *destination, size_t capacity, const char *message) {
    if (capacity) std::snprintf(destination, capacity, "%s", message);
}

} // namespace

struct SfRawSensor;

extern "C" void sf_raw_sensor_free(SfRawSensor *sensor);

/// Unpacked sensor samples plus every LibRaw metadata value the post-unpack
/// numerical stages read. Pointers are owned by the record and released by
/// `sf_raw_sensor_free`.
struct SfRawSensor {
    /// Compacted active-area samples: `channels` floats per site, row-major,
    /// `width` sites per row.
    float *samples;
    uint64_t samples_length;
    /// One byte per CFA position, values 0..=3, `cfa_rows × cfa_columns`
    /// repeating tile indexed by active-area coordinates.
    unsigned char *cfa;
    unsigned cfa_rows;
    unsigned cfa_columns;
    /// LibRaw `cblack[6..]` repeating black offsets, or null when absent.
    unsigned *black_map;
    unsigned black_map_rows;
    unsigned black_map_columns;
    /// LibRaw's row stride in the unpacked allocation, in sites.
    unsigned allocation_row_stride;
    unsigned allocation_width;
    unsigned allocation_height;

    /// 0 = CFA u16, 1 = interleaved3 u16, 2 = interleaved4 u16,
    /// 3 = CFA float, 4 = interleaved3 float, 5 = interleaved4 float.
    int layout;
    unsigned channels;
    unsigned width;
    unsigned height;
    /// Columns/rows LibRaw copies from the allocation into the active area.
    unsigned copied_width;
    unsigned copied_height;
    unsigned filters;
    unsigned fuji_width;
    int fuji_layout;
    unsigned flip;
    double pixel_aspect;
    int is_foveon;
    unsigned colors;
    unsigned black;
    unsigned maximum;
    unsigned data_maximum;
    unsigned channel_black[4];
    float camera_multipliers[4];
    float default_multipliers[4];
    /// Camera→linear-sRGB matrix as LibRaw's `rgb_cam` (3×4).
    float rgb_camera[3][4];
    /// Camera→XYZ normalized by the D65 white, as `cielab()` builds it.
    float xyz_camera[3][4];
    unsigned short white[64];
    int as_shot_wb_applied;
    int decoder_flags;
    int mix_green;
    int zero_is_bad;
    int raw_color;
    int signed_foveon;
    float float_maximum;
};

namespace {

/// The bytes one sensor site occupies for `layout`.
unsigned site_bytes(int layout, unsigned channels) {
    const unsigned sample_bytes = layout >= 3 ? 4u : 2u;
    return sample_bytes * channels;
}

/// Detect the true repeat of `fcol` over the active area, then copy exactly
/// one period. LibRaw patterns repeat with period ≤ 16 in each axis, and the
/// 48-column window is divisible by every candidate period.
void build_cfa_tile(LibRaw &raw, unsigned char **out_tile, unsigned *out_rows, unsigned *out_columns) {
    std::vector<unsigned char> window(static_cast<size_t>(kCfaWindow) * kCfaWindow);
    for (int row = 0; row < kCfaWindow; ++row)
        for (int col = 0; col < kCfaWindow; ++col)
            window[static_cast<size_t>(row) * kCfaWindow + col] =
                static_cast<unsigned char>(raw.fcol(row, col));

    int rows = kCfaWindow;
    for (int candidate : kCfaPeriods) {
        bool periodic = true;
        for (int row = 0; row < kCfaWindow && periodic; ++row)
            for (int col = 0; col < kCfaWindow; ++col)
                if (window[static_cast<size_t>(row) * kCfaWindow + col] !=
                    window[static_cast<size_t>(row % candidate) * kCfaWindow + col]) {
                    periodic = false;
                    break;
                }
        if (periodic) {
            rows = candidate;
            break;
        }
    }
    int columns = kCfaWindow;
    for (int candidate : kCfaPeriods) {
        bool periodic = true;
        for (int row = 0; row < rows && periodic; ++row)
            for (int col = 0; col < kCfaWindow; ++col)
                if (window[static_cast<size_t>(row) * kCfaWindow + col] !=
                    window[static_cast<size_t>(row) * kCfaWindow + (col % candidate)]) {
                    periodic = false;
                    break;
                }
        if (periodic) {
            columns = candidate;
            break;
        }
    }

    auto *tile = static_cast<unsigned char *>(std::malloc(static_cast<size_t>(rows) * columns));
    if (!tile) throw std::bad_alloc();
    for (int row = 0; row < rows; ++row)
        for (int col = 0; col < columns; ++col)
            tile[static_cast<size_t>(row) * columns + col] =
                window[static_cast<size_t>(row) * kCfaWindow + col];
    *out_tile = tile;
    *out_rows = static_cast<unsigned>(rows);
    *out_columns = static_cast<unsigned>(columns);
}

/// LibRaw's `adjust_bl()`, reproduced through public members.
///
/// `adjust_bl()` is protected, and every public entry point that runs it
/// (`raw2image_ex(do_subtract_black)`, `subtract_black()`) also consumes the
/// result: they zero `color.cblack[]` and `color.black`, overwrite
/// `color.data_maximum` with their own reduction, and rewrite
/// `sizes.width`/`sizes.height` for rotated Fuji sensors. This device path has
/// to export the folded offsets, so the fold happens here instead.
///
/// The transformation is exactly LibRaw's, in LibRaw's order, over the same
/// public members it touches (`imgdata.params.user_black`/`user_cblack`,
/// `imgdata.idata.filters`, `LibRaw::FC`, `imgdata.color.cblack[]`/`black`).
/// Both `LibRaw` instances start from the same constructor defaults
/// (`user_black = -1`, `user_cblack[i] = -1000001`), so the override branches
/// below are inert here and behave exactly like the real call.
void fold_black_levels(LibRaw &raw) {
    auto &params = raw.imgdata.params;
    auto &idata = raw.imgdata.idata;
    auto &color = raw.imgdata.color;

    int clear_repeat = 0;
    if (params.user_black >= 0) {
        color.black = static_cast<unsigned>(params.user_black);
        clear_repeat = 1;
    }
    for (int c = 0; c < 4; ++c) {
        if (params.user_cblack[c] > -1000000) {
            color.cblack[c] = static_cast<unsigned>(params.user_cblack[c]);
            clear_repeat = 1;
        }
    }
    if (clear_repeat)
        color.cblack[4] = color.cblack[5] = 0;

    // Add common part to cblack[] early.
    if (idata.filters > 1000 && (color.cblack[4] + 1) / 2 == 1 &&
        (color.cblack[5] + 1) / 2 == 1) {
        int clrs[4];
        int lastg = -1;
        int gcnt = 0;
        for (int c = 0; c < 4; ++c) {
            clrs[c] = raw.FC(c / 2, c % 2);
            if (clrs[c] == 1) {
                ++gcnt;
                lastg = c;
            }
        }
        if (gcnt > 1 && lastg >= 0)
            clrs[lastg] = 3;
        for (int c = 0; c < 4; ++c)
            color.cblack[clrs[c]] +=
                color.cblack[6 + c / 2 % color.cblack[4] * color.cblack[5] +
                             c % 2 % color.cblack[5]];
        color.cblack[4] = color.cblack[5] = 0;
    } else if (idata.filters <= 1000 && color.cblack[4] == 1 &&
               color.cblack[5] == 1) { // Fuji RAF dng
        for (int c = 0; c < 4; ++c)
            color.cblack[c] += color.cblack[6];
        color.cblack[4] = color.cblack[5] = 0;
    }

    // Remove common part from cblack[].
    int common = static_cast<int>(color.cblack[3]);
    for (int c = 0; c < 3; ++c)
        if (common > static_cast<int>(color.cblack[c]))
            common = static_cast<int>(color.cblack[c]);
    for (int c = 0; c < 4; ++c)
        color.cblack[c] -= static_cast<unsigned>(common);
    color.black += static_cast<unsigned>(common);

    // Now calculate the common part of the cblack[6+] map and move it to black.
    if (color.cblack[4] && color.cblack[5]) {
        common = static_cast<int>(color.cblack[6]);
        for (int c = 1; c < static_cast<int>(color.cblack[4] * color.cblack[5]); ++c)
            if (common > static_cast<int>(color.cblack[6 + c]))
                common = static_cast<int>(color.cblack[6 + c]);
        int nonzero = 0;
        for (int c = 0; c < static_cast<int>(color.cblack[4] * color.cblack[5]); ++c) {
            color.cblack[6 + c] -= static_cast<unsigned>(common);
            if (color.cblack[6 + c])
                ++nonzero;
        }
        color.black += static_cast<unsigned>(common);
        if (!nonzero)
            color.cblack[4] = color.cblack[5] = 0;
    }
    for (int c = 0; c < 4; ++c)
        color.cblack[c] += color.black;
}

} // namespace

extern "C" int sf_raw_unpack_sensor(const unsigned char *bytes, size_t length,
                                    SfRawSensor *result, char *error, size_t error_length) {
    if (!result) {
        copy_message(error, error_length, "Sensor result pointer is null");
        return 1;
    }
    std::memset(result, 0, sizeof(*result));
    try {
        LibRaw raw;
        auto check = [](int code) {
            if (code != LIBRAW_SUCCESS) throw std::runtime_error(libraw_strerror(code));
        };
        check(raw.open_buffer(const_cast<unsigned char *>(bytes), length));
        if (raw.error_count() > 0)
            throw std::runtime_error("Data error or unsupported file format");
        raw.imgdata.rawparams.options &= ~LIBRAW_RAWOPTIONS_CONVERTFLOAT_TO_INT;
        check(raw.unpack());
        if (raw.error_count() > 0)
            throw std::runtime_error("Data error or unsupported file format");

        // Match raw2image_start's metadata restoration without processing pixels.
        std::memcpy(&raw.imgdata.color, &raw.imgdata.rawdata.color, sizeof(raw.imgdata.color));
        std::memcpy(&raw.imgdata.sizes, &raw.imgdata.rawdata.sizes, sizeof(raw.imgdata.sizes));
        std::memcpy(&raw.imgdata.idata, &raw.imgdata.rawdata.iparams, sizeof(raw.imgdata.idata));
        std::memcpy(&raw.get_internal_data_pointer()->internal_output_params,
                    &raw.imgdata.rawdata.ioparams, sizeof(raw.imgdata.rawdata.ioparams));

        const auto &rawdata = raw.imgdata.rawdata;
        const auto &sizes = raw.imgdata.sizes;
        const auto &idata = raw.imgdata.idata;
        const auto &color = raw.imgdata.color;

        int layout = -1;
        unsigned channels = 1;
        const void *base = nullptr;
        if (rawdata.raw_image) {
            layout = 0;
            channels = 1;
            base = rawdata.raw_image;
        } else if (rawdata.color3_image) {
            layout = 1;
            channels = 3;
            base = rawdata.color3_image;
        } else if (rawdata.color4_image) {
            layout = 2;
            channels = 4;
            base = rawdata.color4_image;
        } else if (rawdata.float_image) {
            layout = 3;
            channels = 1;
            base = rawdata.float_image;
        } else if (rawdata.float3_image) {
            layout = 4;
            channels = 3;
            base = rawdata.float3_image;
        } else if (rawdata.float4_image) {
            layout = 5;
            channels = 4;
            base = rawdata.float4_image;
        } else {
            throw std::runtime_error("LibRaw returned no unpacked sensor samples");
        }

        // LibRaw records the authoritative row stride in bytes; it can exceed
        // `raw_width` for decoders that allocate padded or interleaved rows.
        unsigned pitch = sizes.raw_pitch;
        if (!pitch) pitch = sizes.raw_width * site_bytes(layout, channels);
        const unsigned stride = pitch / site_bytes(layout, channels);
        if (!stride) throw std::runtime_error("LibRaw reported an unusable raw row stride");

        // `raw2image_ex` restores metadata and normalizes `flip` to the
        // `flip_index` bit layout. Reproduce that normalization exactly.
        unsigned flip = static_cast<unsigned>(sizes.flip);
        switch ((static_cast<int>(flip) + 3600) % 360) {
        case 270: flip = 5; break;
        case 180: flip = 3; break;
        case 90: flip = 6; break;
        default: break;
        }

        // The reference copies the active area with the CFA channel chosen by
        // `fcol(row, col)` on active-area coordinates.
        const unsigned fuji_width = raw.is_fuji_rotated();
        const bool fuji_layout = raw.get_internal_data_pointer()->unpacker_data.fuji_layout != 0;
        const unsigned copy_width = fuji_width
            ? std::min<unsigned>(fuji_width << unsigned(!fuji_layout), sizes.raw_width - sizes.left_margin)
            : std::min<unsigned>(sizes.width, sizes.raw_width > sizes.left_margin
                                                ? sizes.raw_width - sizes.left_margin : 0u);
        const unsigned copy_height = fuji_width
            ? sizes.raw_height - 2u * sizes.top_margin
            : std::min<unsigned>(sizes.height, sizes.raw_height > sizes.top_margin
                                                 ? sizes.raw_height - sizes.top_margin : 0u);

        // `adjust_bl()` is what `raw2image_ex()` runs immediately before the
        // copy: it folds the repeating black map into the per-channel offsets
        // and moves the common component into `black`. It is protected and has
        // no public entry point that leaves `imgdata.color` intact, so
        // `fold_black_levels` reproduces it below through the same public
        // members it reads and writes.
        if (layout < 3) fold_black_levels(raw);

        const unsigned sample_width = fuji_width ? copy_width : sizes.width;
        const unsigned sample_height = fuji_width ? copy_height : sizes.height;
        const size_t count = static_cast<size_t>(sample_width) * sample_height * channels;
        if (count == 0) throw std::runtime_error("RAW sensor has no active image");
        auto *samples = static_cast<float *>(std::malloc(count * sizeof(float)));
        if (!samples) throw std::bad_alloc();
        // The failure handlers own allocations before subsequent work can throw.
        result->samples = samples;
        std::memset(samples, 0, count * sizeof(float));

        const auto *u16 = static_cast<const unsigned short *>(base);
        const auto *f32 = static_cast<const float *>(base);
        for (unsigned row = 0; row < copy_height; ++row) {
            const size_t source_row = static_cast<size_t>(row + sizes.top_margin) * stride;
            const size_t target_row = static_cast<size_t>(row) * sample_width;
            for (unsigned col = 0; col < copy_width; ++col) {
                const size_t source = source_row + col + sizes.left_margin;
                const size_t target = (target_row + col) * channels;
                for (unsigned channel = 0; channel < channels; ++channel) {
                    const size_t index = source * channels + channel;
                    samples[target + channel] =
                        layout >= 3 ? f32[index] : static_cast<float>(u16[index]);
                }
            }
        }

        unsigned char *cfa = nullptr;
        unsigned cfa_rows = 0;
        unsigned cfa_columns = 0;
        build_cfa_tile(raw, &cfa, &cfa_rows, &cfa_columns);
        result->cfa = cfa;

        unsigned *black_map = nullptr;
        unsigned black_map_rows = 0;
        unsigned black_map_columns = 0;
        if (color.cblack[4] && color.cblack[5]) {
            black_map_rows = color.cblack[4];
            black_map_columns = color.cblack[5];
            const size_t entries = static_cast<size_t>(black_map_rows) * black_map_columns;
            black_map = static_cast<unsigned *>(std::malloc(entries * sizeof(unsigned)));
            if (!black_map) throw std::bad_alloc();
            result->black_map = black_map;
            std::memcpy(black_map, &color.cblack[6], entries * sizeof(unsigned));
        }

        result->samples_length = count;
        result->cfa_rows = cfa_rows;
        result->cfa_columns = cfa_columns;
        result->black_map_rows = black_map_rows;
        result->black_map_columns = black_map_columns;
        result->allocation_row_stride = stride;
        result->allocation_width = std::max<unsigned>(sizes.raw_width, sizes.width + sizes.left_margin);
        result->allocation_height = std::max<unsigned>(sizes.raw_height, sizes.height + sizes.top_margin);
        result->layout = layout;
        result->channels = channels;
        result->width = sizes.width;
        result->height = sizes.height;
        result->copied_width = copy_width;
        result->copied_height = copy_height;
        result->filters = idata.filters;
        result->fuji_width = raw.is_fuji_rotated();
        // `unpacker_data` is protected; the public accessor returns the same
        // `libraw_internal_data_t` member.
        result->fuji_layout = static_cast<int>(
            raw.get_internal_data_pointer()->unpacker_data.fuji_layout);
        result->flip = flip;
        result->pixel_aspect = sizes.pixel_aspect;
        result->is_foveon = idata.is_foveon;
        result->colors = static_cast<unsigned>(idata.colors);
        result->black = color.black;
        result->maximum = color.maximum;
        result->data_maximum = color.data_maximum;
        for (int c = 0; c < 4; ++c) {
            result->channel_black[c] = color.cblack[c];
            result->camera_multipliers[c] = color.cam_mul[c];
            result->default_multipliers[c] = color.pre_mul[c];
        }
        for (int i = 0; i < 3; ++i)
            for (int j = 0; j < 4; ++j) {
                result->rgb_camera[i][j] = color.rgb_cam[i][j];
                // `cielab()` accumulates each term after a `float` cast; the
                // product and division round in double first.
                float value = 0.0f;
                if (j < idata.colors)
                    for (int k = 0; k < 3; ++k)
                        value += static_cast<float>(
                            kXyzRgb[i][k] * static_cast<double>(color.rgb_cam[k][j]) /
                            static_cast<double>(kD65White[i]));
                result->xyz_camera[i][j] = value;
            }
        std::memcpy(result->white, color.white, sizeof(result->white));
        result->as_shot_wb_applied = color.as_shot_wb_applied;

        libraw_decoder_info_t decoder_info;
        raw.get_decoder_info(&decoder_info);
        result->decoder_flags = static_cast<int>(decoder_info.decoder_flags);
        result->mix_green = static_cast<int>(raw.get_internal_data_pointer()->internal_output_params.mix_green);
        result->zero_is_bad = static_cast<int>(raw.get_internal_data_pointer()->internal_output_params.zero_is_bad);
        result->raw_color = static_cast<int>(raw.get_internal_data_pointer()->internal_output_params.raw_color);
        result->signed_foveon = idata.is_foveon && std::strcmp(raw.unpack_function_name(), "x3f_load_raw()") == 0;
        result->float_maximum = color.fmaximum;
        return 0;
    } catch (const std::exception &e) {
        sf_raw_sensor_free(result);
        copy_message(error, error_length, e.what());
        return 1;
    } catch (...) {
        sf_raw_sensor_free(result);
        copy_message(error, error_length, "Unknown LibRaw failure");
        return 1;
    }
}

extern "C" void sf_raw_sensor_free(SfRawSensor *sensor) {
    if (!sensor) return;
    std::free(sensor->samples);
    std::free(sensor->cfa);
    std::free(sensor->black_map);
    std::memset(sensor, 0, sizeof(*sensor));
}

// Synthetic native boundary for regression probes: invoke the linked LibRaw
// methods rather than duplicating their geometry arithmetic in a CPU test.
namespace {
class GeometryReference : public LibRaw {
public:
    using LibRaw::fuji_rotate;
    using LibRaw::stretch;
    using LibRaw::xtrans_interpolate;
};
}
extern "C" int sf_raw_geometry_reference(const float* input, unsigned width, unsigned height,
                                         unsigned fuji_width, double aspect, unsigned colors,
                                         float** pixels, unsigned* out_width, unsigned* out_height) {
    *pixels = nullptr;
    try {
        GeometryReference raw;
        raw.imgdata.sizes.width = width;
        raw.imgdata.sizes.height = height;
        raw.imgdata.sizes.pixel_aspect = aspect;
        raw.imgdata.idata.colors = colors;
        raw.get_internal_data_pointer()->internal_output_params.fuji_width = fuji_width;
        raw.imgdata.image = static_cast<unsigned short(*)[4]>(std::calloc(size_t(width) * height, 8));
        if (!raw.imgdata.image) throw std::bad_alloc();
        for (size_t i = 0; i < size_t(width) * height * 4; ++i)
            raw.imgdata.image[i / 4][i % 4] = static_cast<unsigned short>(input[i]);
        raw.fuji_rotate();
        raw.stretch();
        *out_width = raw.imgdata.sizes.width;
        *out_height = raw.imgdata.sizes.height;
        const size_t count = size_t(*out_width) * *out_height * 4;
        float* out = static_cast<float*>(std::malloc(count * sizeof(float)));
        if (!out) throw std::bad_alloc();
        for (size_t i = 0; i < count; ++i) out[i] = raw.imgdata.image[i / 4][i % 4];
        *pixels = out;
        return 0;
    } catch (...) { return 1; }
}

#include "float_reference.cpp"

extern "C" int sf_raw_xtrans_reference(const unsigned* input, unsigned width, unsigned height,
                                        const unsigned char* pattern, float** output) {
    *output = nullptr;
    try {
        GeometryReference raw;
        raw.imgdata.sizes.width = raw.imgdata.sizes.iwidth = width;
        raw.imgdata.sizes.height = raw.imgdata.sizes.iheight = height;
        raw.imgdata.idata.filters = 9;
        raw.imgdata.idata.colors = 3;
        std::memcpy(raw.imgdata.idata.xtrans, pattern, 36);
        std::memset(raw.imgdata.color.rgb_cam, 0, sizeof(raw.imgdata.color.rgb_cam));
        raw.imgdata.image = static_cast<unsigned short(*)[4]>(std::calloc(size_t(width + 6) * (height + 6), 8));
        if (!raw.imgdata.image) throw std::bad_alloc();
        for (size_t i = 0; i < size_t(width) * height * 4; ++i)
            raw.imgdata.image[i / 4][i % 4] = static_cast<unsigned short>(input[i]);
        {
            const OrderedXTrans ordered(raw.imgdata.idata.filters);
            raw.xtrans_interpolate(3);
        }
        float* values = static_cast<float*>(std::malloc(size_t(width) * height * 4 * sizeof(float)));
        if (!values) throw std::bad_alloc();
        for (size_t i = 0; i < size_t(width) * height * 4; ++i) values[i] = raw.imgdata.image[i / 4][i % 4];
        *output = values;
        return 0;
    } catch (...) { return 1; }
}

// Deterministic boundary check, not a probabilistic assertion that a native
// race happens to occur. The caller's setting is restored on every exit.
extern "C" int sf_raw_ordered_xtrans_threading_reference() {
#if defined(SF_RAW_OPENMP)
    const int original = omp_get_max_threads();
    struct Restore {
        int value;
        ~Restore() { omp_set_num_threads(value); }
    } restore{original};
    for (int threads : {2, 3}) {
        omp_set_num_threads(threads);
        {
            const OrderedXTrans unchanged(0x94949494u);
            if (omp_get_max_threads() != threads) return 1;
        }
        if (omp_get_max_threads() != threads) return 2;
        {
            const OrderedXTrans ordered(9);
            if (omp_get_max_threads() != 1) return 3;
            { const OrderedXTrans nested(9); }
            if (omp_get_max_threads() != 1) return 4;
        }
        if (omp_get_max_threads() != threads) return 5;
        try {
            const OrderedXTrans ordered(9);
            throw std::runtime_error("exercise ordered X-Trans unwinding");
        } catch (const std::runtime_error&) {}
        if (omp_get_max_threads() != threads) return 6;
    }
#endif
    return 0;
}

extern "C" int sf_raw_xtrans_threaded_reference(const unsigned* input, unsigned width, unsigned height,
                                                 const unsigned char* pattern, int threads, float** output) {
#if defined(SF_RAW_OPENMP)
    const int original = omp_get_max_threads();
    struct Restore {
        int value;
        ~Restore() { omp_set_num_threads(value); }
    } restore{original};
    omp_set_num_threads(threads);
#else
    (void)threads;
#endif
    const int status = sf_raw_xtrans_reference(input, width, height, pattern, output);
#if defined(SF_RAW_OPENMP)
    if (omp_get_max_threads() != threads) return 2;
#endif
    return status;
}
