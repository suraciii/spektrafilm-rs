#include <libraw/libraw.h>
#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <stdexcept>

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
        check(raw.dcraw_process());
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
