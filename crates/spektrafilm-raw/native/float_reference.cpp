// Synthetic native numerical boundary, independent of file reconstruction.
// Included by raw.cpp so it links the exact selected LibRaw implementation.
#include <libraw/libraw.h>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <new>
#include <stdexcept>

namespace {
class SfFloatReference : public LibRaw {
public:
    void prepare(const float *input, unsigned count, unsigned channels,
                 float fmaximum, const unsigned *metadata,
                 const unsigned *black_map) {
        imgdata.sizes.raw_width = static_cast<unsigned short>(count / channels);
        imgdata.sizes.raw_height = 1;
        libraw_internal_data.unpacker_data.tiff_samples = channels;
        imgdata.idata.filters = metadata[0];
        imgdata.color.black = metadata[1];
        imgdata.color.maximum = metadata[2];
        imgdata.color.fmaximum = fmaximum;
        std::memset(imgdata.color.cblack, 0, sizeof(imgdata.color.cblack));
        for (unsigned c = 0; c < 4; ++c) imgdata.color.cblack[c] = metadata[3 + c];
        imgdata.color.cblack[4] = metadata[7];
        imgdata.color.cblack[5] = metadata[8];
        const unsigned map_count = metadata[7] * metadata[8];
        for (unsigned i = 0; i < map_count; ++i) imgdata.color.cblack[6 + i] = black_map[i];
        imgdata.params.user_black = -1;
        for (unsigned c = 0; c < 4; ++c) imgdata.params.user_cblack[c] = -1000001;
        float *values = static_cast<float *>(std::malloc(static_cast<size_t>(count) * sizeof(float)));
        if (!values) throw std::bad_alloc();
        std::memcpy(values, input, static_cast<size_t>(count) * sizeof(float));
        imgdata.rawdata.raw_alloc = values;
        if (channels == 1) imgdata.rawdata.float_image = values;
        else if (channels == 3) imgdata.rawdata.float3_image = reinterpret_cast<float (*)[3]>(values);
        else imgdata.rawdata.float4_image = reinterpret_cast<float (*)[4]>(values);
    }
    void fold_black() { adjust_bl(); }
};
} // namespace

// Metadata input: filters, black, maximum, cblack[0..4], repeat rows/columns.
// Output has the same shape, except word 0 contains the native fnorm f32 bits.
// A folded/cleared output map is zero-filled over its original dimensions.
extern "C" int sf_raw_float_reference(const float *input, unsigned count,
                                      unsigned channels, float fmaximum,
                                      const unsigned *metadata,
                                      const unsigned *black_map, float **output,
                                      unsigned *out_metadata,
                                      unsigned *out_black_map) {
    if (!output) return 1;
    *output = nullptr;
    if (!input || !metadata || !out_metadata || count == 0 ||
        (channels != 1 && channels != 3 && channels != 4) || count % channels ||
        count / channels > std::numeric_limits<unsigned short>::max()) return 1;
    const uint64_t map_count = uint64_t(metadata[7]) * metadata[8];
    if (map_count > LIBRAW_CBLACK_SIZE - 6 ||
        (map_count && (!black_map || !out_black_map))) return 1;
    try {
        SfFloatReference raw;
        raw.prepare(input, count, channels, fmaximum, metadata, black_map);
        raw.convertFloatToInt();
        raw.fold_black();
        const unsigned short *values = channels == 1 ? raw.imgdata.rawdata.raw_image
            : channels == 3 ? reinterpret_cast<const unsigned short *>(raw.imgdata.rawdata.color3_image)
            : reinterpret_cast<const unsigned short *>(raw.imgdata.rawdata.color4_image);
        float *result = static_cast<float *>(std::malloc(static_cast<size_t>(count) * sizeof(float)));
        if (!result) throw std::bad_alloc();
        for (unsigned i = 0; i < count; ++i) result[i] = static_cast<float>(values[i]);
        const auto &color = raw.imgdata.color;
        std::memcpy(out_metadata, &color.fnorm, sizeof(float));
        out_metadata[1] = color.black;
        out_metadata[2] = color.maximum;
        for (unsigned c = 0; c < 4; ++c) out_metadata[3 + c] = color.cblack[c];
        out_metadata[7] = color.cblack[4];
        out_metadata[8] = color.cblack[5];
        const bool has_map = color.cblack[4] && color.cblack[5];
        for (unsigned i = 0; i < map_count; ++i) out_black_map[i] = has_map ? color.cblack[6 + i] : 0;
        *output = result;
        return 0;
    } catch (...) {
        return 2;
    }
}
