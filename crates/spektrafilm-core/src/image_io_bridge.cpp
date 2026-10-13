#include <OpenImageIO/imageio.h>
#include <OpenImageIO/half.h>
#include <OpenImageIO/filesystem.h>
#include <exiv2/exiv2.hpp>
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstdint>
#include <cstring>
#include <ctime>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
struct Metadata { Exiv2::ExifData exif; Exiv2::IptcData iptc; Exiv2::XmpData xmp; };
std::once_flag xmp_initialization;
std::mutex xmp_namespace_mutex;
// Plugins can return success while their underlying writes fail. Track the
// stream independently, including buffered errors exposed only at finalization.
class CheckedOutput : public OIIO::Filesystem::IOFile {
public:
    explicit CheckedOutput(FILE* file) : IOFile(file, Write) {}
    const char* proxytype() const override { return "spektrafilm-checked-file"; }
    void close() override { m_mode = Closed; }
    size_t write(const void* data, size_t size) override {
        const auto written = IOFile::write(data, size);
        if (written != size) failed = true;
        return written;
    }
    size_t pwrite(const void* data, size_t size, int64_t offset) override {
        const auto written = IOFile::pwrite(data, size, offset);
        if (written != size) failed = true;
        return written;
    }
    std::atomic<bool> failed{false};
};
struct FileCloser {
    void operator()(FILE* file) const { if (file) std::fclose(file); }
};
// OIIO probes every registered plugin when opening a file, including its
// LibRaw-based reader. Probing bytes that are not a supported container can
// corrupt the heap inside that plugin and abort the process, so the container
// is identified from its own magic bytes before OIIO is asked to open it.
bool supported_container(const std::string& path, std::string& reason) {
    std::unique_ptr<FILE, FileCloser> file(OIIO::Filesystem::fopen(path.c_str(), "rb"));
    if (!file) { reason = "Cannot open image file"; return false; }
    unsigned char magic[16] = {0};
    const auto read = std::fread(magic, 1, sizeof(magic), file.get());
    if (read < 4) { reason = "File is too small to be an image"; return false; }
    // JPEG, PNG, TIFF (both byte orders) and OpenEXR.
    if (magic[0] == 0xFF && magic[1] == 0xD8 && magic[2] == 0xFF) return true;
    if (magic[0] == 0x89 && magic[1] == 'P' && magic[2] == 'N' && magic[3] == 'G') return true;
    if ((magic[0] == 'I' && magic[1] == 'I' && magic[2] == 0x2A && magic[3] == 0x00) ||
        (magic[0] == 'M' && magic[1] == 'M' && magic[2] == 0x00 && magic[3] == 0x2A)) return true;
    if (magic[0] == 0x76 && magic[1] == 0x2F && magic[2] == 0x31 && magic[3] == 0x01) return true;
    reason = "Unrecognized image container";
    return false;
}
void lock_xmp_namespace(void* pointer, bool lock) {
    auto& mutex = *static_cast<std::mutex*>(pointer);
    if (lock) mutex.lock();
    else mutex.unlock();
}
void initialize_xmp() {
    // Exiv2's lazy XMP initialization is not thread-safe. Initialize once
    // before metadata reads/writes, and protect later namespace registration
    // with the callback supported by Exiv2 rather than serializing image I/O.
    std::call_once(xmp_initialization, [] {
        if (!Exiv2::XmpParser::initialize(lock_xmp_namespace, &xmp_namespace_mutex))
            throw std::runtime_error("Exiv2 XMP toolkit initialization failed");
    });
}
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
        initialize_xmp();
        auto image = Exiv2::ImageFactory::open(path);
        if (!image.get()) return nullptr;
        image->readMetadata();
        return new Metadata{image->exifData(), image->iptcData(), image->xmpData()};
    } catch (...) { return nullptr; }
}
int sf_image_load(const char* path, unsigned* width, unsigned* height, double** samples, char** err) {
    try {
        std::string container_reason;
        if (!supported_container(path, container_reason)) {
            error(err, container_reason);
            return 0;
        }
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
                  int depth, int format, int jpeg_quality, int jpeg_subsampling,
                  int compression, const char* color_space, bool export_metadata,
                  const unsigned char* icc, size_t icc_len, char** err) {
    try {
        // format: JPEG=0, PNG=1, TIFF=2, EXR=3. Quantization truncates, like numpy astype.
        OIIO::TypeDesc type = OIIO::TypeDesc::UINT8;
        if (format == 2) type = depth == 8 ? OIIO::TypeDesc::UINT8 : depth == 16 ? OIIO::TypeDesc::UINT16 : OIIO::TypeDesc::FLOAT;
        if (format == 3) type = depth == 16 ? OIIO::TypeDesc::HALF : OIIO::TypeDesc::FLOAT;
        OIIO::ImageSpec spec(width, height, 3, type);
        const bool record_density = std::strcmp(color_space, "Fotufilm record density") == 0;
        if (record_density) {
            spec.channelnames = {"C", "M", "Y"};
            spec.attribute("spektrafilm:sample_domain", "Fotufilm gross record optical density");
        }
        if (format == 0) {
            if (jpeg_quality > 0) spec.attribute("CompressionQuality", jpeg_quality);
            if (jpeg_subsampling != 0)
                spec.attribute("jpeg:subsampling", jpeg_subsampling == 420 ? "4:2:0" : "4:4:4");
        }
        if (format == 2 || format == 3)
            spec.attribute("Compression", compression == 0 ? "none" : "zip");
        if (format == 2)
            spec.attribute("tiff:write_exif", 0);
        if (format == 3 && export_metadata && !record_density) {
            const float srgb[] = {0.64f, 0.33f, 0.30f, 0.60f, 0.15f, 0.06f, 0.3127f, 0.3290f};
            const float aces[] = {0.7347f, 0.2653f, 0.0f, 1.0f, 0.0001f, -0.0770f, 0.32168f, 0.33767f};
            const float* chromaticities = std::strcmp(color_space, "ACES2065-1") == 0 ? aces : srgb;
            spec.attribute("chromaticities", OIIO::TypeDesc(OIIO::TypeDesc::FLOAT, 8), chromaticities);
        }
        if (icc && icc_len && format != 3)
            spec.attribute("ICCProfile", OIIO::TypeDesc(OIIO::TypeDesc::UINT8, int(icc_len)), icc);
        std::unique_ptr<FILE, FileCloser> file(OIIO::Filesystem::fopen(path, "wb"));
        if (!file) { error(err, std::string("Cannot open image output: ") + std::strerror(errno)); return 0; }
        CheckedOutput proxy(file.get());
        auto output = OIIO::ImageOutput::create(path);
        if (!output) {
            auto message = OIIO::geterror();
            error(err, message.empty() ? "Cannot create image output plugin" : message);
            return 0;
        }
        if (!output->set_ioproxy(&proxy)) {
            error(err, "Image output plugin does not support checked file writes");
            return 0;
        }
        if (!output->open(path, spec)) {
            auto message = output->geterror();
            error(err, message.empty() ? "Cannot open image container" : message);
            return 0;
        }
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
        std::string message = output->geterror();
        if (!ok && message.empty()) message = "Native image data write failed";
        const bool closed = output->close();
        const auto close_message = output->geterror();
        if (!close_message.empty()) {
            if (!message.empty()) message += "; ";
            message += close_message;
        }
        if (!closed && message.empty()) message = "Native image container finalization failed";
        output.reset();
        const bool flushed = std::fflush(file.get()) == 0;
        const bool stream_failed = std::ferror(file.get()) != 0;
        proxy.close();
        // Close the staged file before the caller publishes it: Windows rejects
        // the publishing rename while this handle is open.
        const bool stream_closed = std::fclose(file.release()) == 0;
        if (proxy.failed || !flushed || stream_failed || !stream_closed) {
            if (!message.empty()) message += "; ";
            message += "Native image file write/finalization failed";
        }
        if (!ok || !closed || !message.empty()) { error(err, message); return 0; }
        return 1;
    } catch (const std::exception& ex) { error(err, ex.what()); return 0; }
}
int sf_image_validate(const char* path, char** err) {
    try {
        std::string container_reason;
        if (!supported_container(path, container_reason)) {
            error(err, container_reason);
            return 0;
        }
        auto input = OIIO::ImageInput::open(path);
        if (!input) { error(err, "Image container is unreadable after metadata failure"); return 0; }
        const auto spec = input->spec();
        if (spec.width <= 0 || spec.height <= 0 || spec.nchannels < 3) {
            error(err, "Invalid image container after metadata failure"); return 0;
        }
        std::vector<float> row(size_t(spec.width) * spec.nchannels);
        for (int y = spec.y; y < spec.y + spec.height; ++y) {
            if (!input->read_scanline(y, spec.z, OIIO::TypeDesc::FLOAT, row.data())) {
                auto message = input->geterror();
                error(err, message.empty() ? "Incomplete image after metadata failure" : message); return 0;
            }
        }
        if (!input->close()) { error(err, "Image read finalization failed after metadata failure"); return 0; }
        return 1;
    } catch (const std::exception& ex) { error(err, ex.what()); return 0; }
}
int sf_metadata_write(const char* path, const void* source, unsigned width, unsigned height,
                      const char* space, bool encoded, char** err) {
    try {
        initialize_xmp();
        auto image = Exiv2::ImageFactory::open(path);
        image->readMetadata();
        if (source) {
            auto metadata = static_cast<const Metadata*>(source);
            image->setExifData(metadata->exif); image->setIptcData(metadata->iptc); image->setXmpData(metadata->xmp);
        }
        auto& exif = image->exifData();
        exif["Exif.Image.Orientation"] = uint16_t(1);
        char timestamp[20]; auto now = std::time(nullptr); std::tm local{};
#ifdef _WIN32
        const bool time_ok = localtime_s(&local, &now) == 0;
#else
        const bool time_ok = localtime_r(&now, &local) != nullptr;
#endif
        if (!time_ok) throw std::runtime_error("Cannot obtain metadata timestamp");
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
