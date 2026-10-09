#include <OpenImageIO/imageio.h>
#include <OpenImageIO/half.h>
#include <exiv2/exiv2.hpp>
#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <cstdint>
#include <cstring>
#include <ctime>
#include <memory>
#include <string>
#include <vector>

namespace {
struct Metadata { Exiv2::ExifData exif; Exiv2::IptcData iptc; Exiv2::XmpData xmp; };
// Imath's half constructor and OIIO's conversion both pass through float.
// Round binary64 directly, like numpy astype(float16), using integer bits so
// halfway cases use ties-to-even regardless of the floating-point environment.
uint16_t double_to_half_bits(double value) {
    uint64_t bits;
    std::memcpy(&bits, &value, sizeof(bits));
    const uint16_t sign = uint16_t((bits >> 48) & 0x8000);
    const unsigned exponent_bits = unsigned((bits >> 52) & 0x7ff);
    const uint64_t fraction = bits & 0x000fffffffffffffULL;
    if (exponent_bits == 0x7ff)
        return uint16_t(sign | 0x7c00 | (fraction ? ((fraction >> 42) | 0x0200) : 0));
    const int exponent = int(exponent_bits) - 1023;
    if (exponent < -25) return sign;
    if (exponent > 15) return uint16_t(sign | 0x7c00);
    const uint64_t significand = fraction | (uint64_t(1) << 52);
    const unsigned shift = exponent < -14 ? unsigned(28 - exponent) : 42;
    uint64_t rounded = significand >> shift;
    const uint64_t remainder = significand & ((uint64_t(1) << shift) - 1);
    const uint64_t midpoint = uint64_t(1) << (shift - 1);
    if (remainder > midpoint || (remainder == midpoint && (rounded & 1))) ++rounded;
    // A rounding carry naturally crosses the subnormal/normal boundary or
    // turns the largest finite exponent into infinity at the overflow midpoint.
    const uint16_t magnitude = exponent < -14 ? uint16_t(rounded)
        : uint16_t(((exponent + 15) << 10) + rounded - 1024);
    return uint16_t(sign | magnitude);
}
void error(char** target, const std::string& text) {
    if (!target) return;
    *target = static_cast<char*>(std::malloc(text.size() + 1));
    if (*target) std::memcpy(*target, text.c_str(), text.size() + 1);
}
}
extern "C" {
void sf_io_free(void* pointer) { std::free(pointer); }
void sf_metadata_free(void* pointer) { delete static_cast<Metadata*>(pointer); }
void* sf_metadata_read(const char* path) {
    try {
        auto image = Exiv2::ImageFactory::open(path);
        if (!image.get()) return nullptr;
        image->readMetadata();
        return new Metadata{image->exifData(), image->iptcData(), image->xmpData()};
    } catch (...) { return nullptr; }
}
int sf_image_load(const char* path, unsigned* width, unsigned* height, double** samples, char** err) {
    try {
        auto input = OIIO::ImageInput::open(path);
        if (!input) { error(err, OIIO::geterror()); return 0; }
        auto spec = input->spec();
        if (spec.width <= 0 || spec.height <= 0 || spec.nchannels < 1) { error(err, "Invalid image dimensions/channels"); return 0; }
        std::vector<double> native(size_t(spec.width) * spec.height * spec.nchannels);
        if (!input->read_image(0, 0, 0, spec.nchannels, OIIO::TypeDesc::DOUBLE, native.data())) { error(err, input->geterror()); return 0; }
        if (!input->close()) { error(err, input->geterror()); return 0; }
        auto count = size_t(spec.width) * spec.height * 3;
        auto rgb = static_cast<double*>(std::malloc(count * sizeof(double)));
        if (!rgb) throw std::bad_alloc();
        // Alpha is not composited: the simulation consumes RGB samples only.
        for (size_t p = 0; p < count / 3; ++p)
            for (int c = 0; c < 3; ++c)
                rgb[p * 3 + c] = native[p * spec.nchannels + (spec.nchannels < 3 ? 0 : c)];
        *width = spec.width; *height = spec.height; *samples = rgb;
        return 1;
    } catch (const std::exception& ex) { error(err, ex.what()); return 0; }
}
int sf_image_save(const char* path, unsigned width, unsigned height, const double* samples,
                  int depth, int format, int jpeg_quality,
                  const unsigned char* icc, size_t icc_len, char** err) {
    try {
        // format: JPEG=0, PNG=1, TIFF=2, EXR=3. Quantization truncates, like numpy astype.
        OIIO::TypeDesc type = OIIO::TypeDesc::UINT8;
        if (format == 2) type = depth == 8 ? OIIO::TypeDesc::UINT8 : depth == 16 ? OIIO::TypeDesc::UINT16 : OIIO::TypeDesc::FLOAT;
        if (format == 3) type = depth == 16 ? OIIO::TypeDesc::HALF : OIIO::TypeDesc::FLOAT;
        OIIO::ImageSpec spec(width, height, 3, type);
        if (format == 0 && jpeg_quality > 0)
            spec.attribute("CompressionQuality", jpeg_quality);
        if (format == 2) spec.attribute("Compression", "zip");
        if (icc && icc_len && format != 3)
            spec.attribute("ICCProfile", OIIO::TypeDesc(OIIO::TypeDesc::UINT8, int(icc_len)), icc);
        auto output = OIIO::ImageOutput::create(path);
        if (!output) { error(err, OIIO::geterror()); return 0; }
        if (!output->open(path, spec)) { error(err, output->geterror()); return 0; }
        size_t count = size_t(width) * height * 3;
        bool ok;
        if (type == OIIO::TypeDesc::UINT8) {
            std::vector<uint8_t> data(count);
            for (size_t i=0; i<count; ++i) data[i] = uint8_t(std::isnan(samples[i]) ? 0 : std::clamp(samples[i], 0.0, 1.0) * 255.0);
            ok = output->write_image(type, data.data());
        } else if (type == OIIO::TypeDesc::UINT16) {
            std::vector<uint16_t> data(count);
            for (size_t i=0; i<count; ++i) data[i] = uint16_t(std::isnan(samples[i]) ? 0 : std::clamp(samples[i], 0.0, 1.0) * 65535.0);
            ok = output->write_image(type, data.data());
        } else if (type == OIIO::TypeDesc::HALF) {
            std::vector<half> data(count);
            for (size_t i=0; i<count; ++i) data[i].setBits(double_to_half_bits(samples[i]));
            ok = output->write_image(type, data.data());
        } else {
            // Float32 exports retain the upstream explicit float32 conversion.
            std::vector<float> data(samples, samples + count);
            ok = output->write_image(OIIO::TypeDesc::FLOAT, data.data());
        }
        std::string message = ok ? "" : output->geterror();
        bool closed = output->close();
        if (!ok || !closed) { error(err, message.empty() ? output->geterror() : message); return 0; }
        return 1;
    } catch (const std::exception& ex) { error(err, ex.what()); return 0; }
}
int sf_metadata_write(const char* path, const void* source, unsigned width, unsigned height,
                      const char* space, bool encoded, char** err) {
    try {
        auto image = Exiv2::ImageFactory::open(path);
        image->readMetadata();
        if (source) {
            auto metadata = static_cast<const Metadata*>(source);
            image->setExifData(metadata->exif); image->setIptcData(metadata->iptc); image->setXmpData(metadata->xmp);
        }
        auto& exif = image->exifData();
        exif["Exif.Image.Orientation"] = uint16_t(1);
        char timestamp[20]; auto now = std::time(nullptr); auto local = *std::localtime(&now);
        std::strftime(timestamp, sizeof(timestamp), "%Y:%m:%d %H:%M:%S", &local);
        exif["Exif.Image.DateTime"] = timestamp;
        exif["Exif.Image.Software"] = "spektrafilm";
        exif["Exif.Photo.PixelXDimension"] = uint32_t(width);
        exif["Exif.Photo.PixelYDimension"] = uint32_t(height);
        std::string color(space);
        exif["Exif.Photo.ColorSpace"] = uint16_t(color == "sRGB" && encoded ? 1 : 65535);
        if (encoded && (color == "sRGB" || color == "Adobe RGB (1998)"))
            exif["Exif.Iop.InteroperabilityIndex"] = color == "sRGB" ? "R98" : "R03";
        else {
            auto stale = exif.findKey(Exiv2::ExifKey("Exif.Iop.InteroperabilityIndex"));
            if (stale != exif.end()) exif.erase(stale);
        }
        image->xmpData()["Xmp.photoshop.ICCProfile"] = color + (encoded ? "" : " (linear)");
        image->writeMetadata();
        return 1;
    } catch (const std::exception& ex) { error(err, ex.what()); return 0; }
}
}
