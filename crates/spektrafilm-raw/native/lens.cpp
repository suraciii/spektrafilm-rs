#include <exiv2/exiv2.hpp>
#include <glib.h>
#include <lensfun.h>

#include <cstdint>
#include <algorithm>
#include <charconv>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <cstdlib>
#include <filesystem>
#ifdef _WIN32
#include <windows.h>
#elif defined(__APPLE__)
#include <mach-o/dyld.h>
#else
#include <unistd.h>
#endif
#include <limits>
#include <memory>
#include <set>
#include <stdexcept>
#include <string>
#include <tuple>
#include <utility>
#include <vector>

namespace {

void copy_message(char* destination, size_t capacity, const char* message) noexcept {
    if (destination && capacity) {
        std::snprintf(destination, capacity, "%s", message);
    }
}

struct GlibFree {
    void operator()(void* value) const noexcept { g_free(value); }
};

bool text_space(gunichar character) {
    return g_unichar_isspace(character) || character == 0x85 ||
           (character >= 0x1c && character <= 0x1f);
}

std::string trim_text(const std::string& text) {
    const char* first = text.c_str();
    const char* last = first + text.size();
    while (first < last && text_space(g_utf8_get_char(first))) {
        first = g_utf8_next_char(first);
    }
    while (first < last) {
        const char* previous = g_utf8_find_prev_char(first, last);
        if (!previous || !text_space(g_utf8_get_char(previous))) break;
        last = previous;
    }
    return std::string(first, last);
}

struct LensText {
    std::string normalized;
    std::string compact;
    std::set<std::string> tokens;
};

gunichar lowercase_symbol(gunichar character) {
    // GLib lowercases letters but leaves these Unicode uppercase symbols intact.
    if (character >= 0x2160 && character <= 0x216f) return character + 0x10;
    if (character >= 0x24b6 && character <= 0x24cf) return character + 0x1a;
    return character;
}

LensText lens_text(const char* text) {
    LensText result;
    if (!text) return result;
    std::unique_ptr<char, GlibFree> valid(g_utf8_make_valid(text, -1));
    std::unique_ptr<char, GlibFree> lower(g_utf8_strdown(valid.get(), -1));
    bool pending_space = false;
    for (const char* cursor = lower.get(); *cursor;) {
        const gunichar character = lowercase_symbol(g_utf8_get_char(cursor));
        const char* next = g_utf8_next_char(cursor);
        if (text_space(character)) {
            pending_space = !result.normalized.empty();
        } else {
            if (pending_space) result.normalized.push_back(' ');
            pending_space = false;
            char encoded[6];
            const int length = g_unichar_to_utf8(character, encoded);
            result.normalized.append(encoded, length);
        }
        cursor = next;
    }
    // Python filters the original characters with isalnum(), then lowercases each
    // retained character. In particular, U+0130 retains its combining dot.
    for (const char* cursor = valid.get(); *cursor; cursor = g_utf8_next_char(cursor)) {
        const gunichar character = g_utf8_get_char(cursor);
        const GUnicodeType type = g_unichar_type(character);
        if (!g_unichar_isalnum(character) && type != G_UNICODE_LETTER_NUMBER &&
            type != G_UNICODE_OTHER_NUMBER) continue;
        if (character == 0x130) {
            result.compact += "i\xcc\x87";
        } else {
            char encoded[6];
            const int length = g_unichar_to_utf8(lowercase_symbol(g_unichar_tolower(character)), encoded);
            result.compact.append(encoded, length);
        }
    }
    size_t start = 0;
    while (start < result.normalized.size()) {
        const size_t end = result.normalized.find(' ', start);
        result.tokens.insert(result.normalized.substr(start, end - start));
        if (end == std::string::npos) break;
        start = end + 1;
    }
    return result;
}

struct Metadata {
    std::string make, model, lens_make, lens_model;
    float focal = 0, aperture = 0;
};

Metadata read_metadata(const char* path) {
    try {
        auto image = Exiv2::ImageFactory::open(path);
        if (!image.get()) return {};
        image->readMetadata();
        const auto& exif = image->exifData();
        const auto text = [&](const char* key) {
            const auto found = exif.findKey(Exiv2::ExifKey(key));
            return found == exif.end() ? std::string{} : trim_text(found->toString());
        };
        const auto number = [&](const char* key) {
            const auto found = exif.findKey(Exiv2::ExifKey(key));
            return found == exif.end() ? 0.0f : found->toFloat();
        };
        return {text("Exif.Image.Make"), text("Exif.Image.Model"),
                text("Exif.Photo.LensMake"), text("Exif.Photo.LensModel"),
                number("Exif.Photo.FocalLength"), number("Exif.Photo.FNumber")};
    } catch (...) {
        // The upstream EXIF reader treats unreadable metadata as empty fields.
        return {};
    }
}

using ModelScore = std::tuple<int, int, size_t>;

ModelScore model_score(const LensText& lens, const LensText& exif) {
    size_t common = 0;
    for (const auto& token : exif.tokens) common += lens.tokens.count(token);
    return {lens.normalized == exif.normalized,
            !exif.compact.empty() && lens.compact.find(exif.compact) != std::string::npos,
            common};
}

bool focal_matches(const lfLens& lens, float focal) {
    return focal > 0 && lens.MinFocal <= focal && focal <= lens.MaxFocal;
}

bool aperture_matches(const lfLens& lens, float aperture) {
    // lensfunpy exposes zero-valued aperture bounds as None.
    return aperture > 0 && lens.MinAperture != 0 && lens.MinAperture <= aperture &&
           (lens.MaxAperture == 0 || aperture <= lens.MaxAperture);
}

std::vector<const lfLens*> find_candidates(const lfDatabase& db, const lfCamera* camera,
                                         const Metadata& metadata) {
    using Identity = std::tuple<std::string, std::string, float, float, float, float>;
    const auto query = [&](const char* maker, const char* model, int flags) {
        std::unique_ptr<const lfLens*, GlibFree> found(db.FindLenses(camera, maker, model, flags));
        std::set<Identity> seen;
        std::vector<const lfLens*> candidates;
        if (found) {
            for (auto item = found.get(); *item; ++item) {
                const lfLens* lens = *item;
                const Identity identity{lens_text(lens->Maker).normalized,
                                        lens_text(lens->Model).normalized,
                                        lens->MinFocal, lens->MaxFocal,
                                        lens->MinAperture, lens->MaxAperture};
                if (seen.insert(identity).second) candidates.push_back(lens);
            }
        }
        return candidates;
    };
    const char* maker = metadata.lens_make.empty() ? nullptr : metadata.lens_make.c_str();
    auto candidates = query(maker, metadata.lens_model.c_str(), 0);
    if (!candidates.empty()) return candidates;
    candidates = query(maker, metadata.lens_model.c_str(), LF_SEARCH_LOOSE);
    if (!candidates.empty()) return candidates;
    candidates = query(nullptr, metadata.lens_model.c_str(), LF_SEARCH_LOOSE);
    if (!candidates.empty()) return candidates;
    candidates = query(nullptr, nullptr, LF_SEARCH_LOOSE);
    const LensText model = lens_text(metadata.lens_model.c_str());
    candidates.erase(std::remove_if(candidates.begin(), candidates.end(), [&](const lfLens* lens) {
        return model_score(lens_text(lens->Model), model) == ModelScore{0, 0, 0};
    }), candidates.end());
    return candidates;
}

const lfLens* select_candidate(const std::vector<const lfLens*>& candidates,
                               const Metadata& metadata) {
    const bool filter_focal = std::any_of(candidates.begin(), candidates.end(), [&](const lfLens* lens) {
        return focal_matches(*lens, metadata.focal);
    });
    const LensText model = lens_text(metadata.lens_model.c_str());
    const std::string maker = lens_text(metadata.lens_make.c_str()).normalized;
    using Score = std::tuple<ModelScore, int, int, int, int>;
    const lfLens* best = nullptr;
    Score best_score;
    for (const lfLens* lens : candidates) {
        const bool focal = focal_matches(*lens, metadata.focal);
        if (filter_focal && !focal) continue;
        const Score score{model_score(lens_text(lens->Model), model),
                          !maker.empty() && maker == lens_text(lens->Maker).normalized,
                          focal, aperture_matches(*lens, metadata.aperture), lens->Score};
        // Strict comparison retains the first candidate for equal scores, like Python's stable sort.
        if (!best || score > best_score) {
            best = lens;
            best_score = score;
        }
    }
    return best;
}

std::string float_label(float value) {
    char buffer[64];
    const auto converted = std::to_chars(buffer, buffer + sizeof(buffer), static_cast<double>(value));
    if (converted.ec != std::errc{}) throw std::runtime_error("Cannot format lens metadata");
    std::string result(buffer, converted.ptr);
    if (result.find_first_of(".eE") == std::string::npos && std::isfinite(value)) result += ".0";
    return result;
}

float bilinear(const float* rgb, unsigned width, unsigned height, unsigned channel,
               float coordinate_x, float coordinate_y) {
    // scipy.ndimage.map_coordinates(order=1, mode="nearest") extends edge pixels.
    if (std::isnan(coordinate_x) || std::isnan(coordinate_y)) return 0;
    const double x = std::clamp(static_cast<double>(coordinate_x), 0.0, static_cast<double>(width - 1));
    const double y = std::clamp(static_cast<double>(coordinate_y), 0.0, static_cast<double>(height - 1));
    const size_t x0 = static_cast<size_t>(x), y0 = static_cast<size_t>(y);
    const size_t x1 = std::min(x0 + 1, static_cast<size_t>(width - 1));
    const size_t y1 = std::min(y0 + 1, static_cast<size_t>(height - 1));
    const double dx = x - x0, dy = y - y0;
    const auto pixel = [&](size_t px, size_t py) { return rgb[(py * width + px) * 3 + channel]; };
    return static_cast<float>(pixel(x0, y0) * (1 - dx) * (1 - dy) +
                              pixel(x1, y0) * dx * (1 - dy) +
                              pixel(x0, y1) * (1 - dx) * dy + pixel(x1, y1) * dx * dy);
}

} // namespace

namespace {
std::filesystem::path executable_directory() {
#ifdef _WIN32
    std::vector<wchar_t> buffer(32768);
    const DWORD length = GetModuleFileNameW(nullptr, buffer.data(), static_cast<DWORD>(buffer.size()));
    if (!length || length == buffer.size()) return {};
    return std::filesystem::path(std::wstring(buffer.data(), length)).parent_path();
#elif defined(__APPLE__)
    uint32_t length = 0;
    _NSGetExecutablePath(nullptr, &length);
    std::vector<char> buffer(length);
    if (_NSGetExecutablePath(buffer.data(), &length)) return {};
    return std::filesystem::path(buffer.data()).parent_path();
#else
    std::vector<char> buffer(4096);
    const auto length = readlink("/proc/self/exe", buffer.data(), buffer.size());
    if (length <= 0 || static_cast<size_t>(length) == buffer.size()) return {};
    return std::filesystem::path(std::string(buffer.data(), length)).parent_path();
#endif
}
lfError load_database(lfDatabase& database) {
    if (const char* configured = std::getenv("SPEKTRAFILM_LENSFUN_DATABASE"); configured && *configured) {
        return database.LoadDirectory(configured) ? LF_NO_ERROR : LF_NO_DATABASE;
    }
    const auto executable = executable_directory();
    if (!executable.empty()) {
        for (const auto& path : {executable / "lensfun", executable / "../Resources/lensfun"}) {
            if (std::filesystem::is_directory(path)) {
                return database.LoadDirectory(path.string().c_str()) ? LF_NO_ERROR : LF_NO_DATABASE;
            }
        }
    }
    return database.Load();
}

std::pair<const lfCamera*, const lfLens*> resolve_model(lfDatabase& database,
                                                       const Metadata& metadata) {
    if (metadata.lens_model.empty()) return {nullptr, nullptr};
    const lfError status = load_database(database);
    if (status != LF_NO_ERROR) {
        throw std::runtime_error("Cannot load the Lensfun database (error " + std::to_string(status) +
                                 "). Install the Lensfun lens database package or update it with lensfun-update-data.");
    }
    std::unique_ptr<const lfCamera*, GlibFree> cameras(
        database.FindCamerasExt(metadata.make.c_str(), metadata.model.c_str(), LF_SEARCH_LOOSE));
    if (!cameras || !*cameras) return {nullptr, nullptr};
    const lfCamera* camera = *cameras;
    const auto candidates = find_candidates(database, camera, metadata);
    if (candidates.empty()) return {camera, nullptr};
    return {camera, select_candidate(candidates, metadata)};
}

char* model_identity(const lfCamera* camera, const lfLens* lens) {
    std::unique_ptr<char, GlibFree> model;
    if (!lens) {
        model.reset(g_strdup("no-matching-lens-correction"));
    } else {
        const lfCamera* cameras[] = {camera, nullptr};
        const lfLens* lenses[] = {lens, nullptr};
        model.reset(lfDatabase::Save(nullptr, cameras, lenses));
    }
    if (!model) throw std::bad_alloc();
    char* identity = g_strdup_printf("lensfun:%u\n%s", static_cast<unsigned>(LF_VERSION), model.get());
    if (!identity) throw std::bad_alloc();
    return identity;
}
}

extern "C" int sf_raw_correct_lens(const char* path, float* rgb, unsigned width, unsigned height,
                                    char* summary, size_t summary_len, char** identity,
                                    char* error, size_t error_len) {
    copy_message(summary, summary_len, "");
    copy_message(error, error_len, "");
    *identity = nullptr;
    try {
        if (!path || !rgb || width == 0 || height == 0 ||
            width > static_cast<unsigned>(std::numeric_limits<int>::max() / (3 * sizeof(float))) ||
            height > static_cast<unsigned>(std::numeric_limits<int>::max()) ||
            static_cast<size_t>(height) > std::numeric_limits<size_t>::max() / width / 6 / sizeof(float)) {
            throw std::invalid_argument("Lens correction requires a path and a nonempty float32 RGB image within native size limits");
        }
        const Metadata metadata = read_metadata(path);
        lfDatabase database;
        const auto [camera, lens] = resolve_model(database, metadata);
        std::unique_ptr<char, GlibFree> accepted_identity(model_identity(camera, lens));
        if (!lens) {
            *identity = accepted_identity.release();
            return 0;
        }
        const std::string label = lens->Model && *lens->Model ? lens->Model : metadata.lens_model;
        const std::string information = label + " @ " + float_label(metadata.focal) +
                                        "mm f/" + float_label(metadata.aperture);
        lfModifier modifier(lens, camera->CropFactor, static_cast<int>(width), static_cast<int>(height));
        // lensfunpy 1.18.0 defaults: distance 1000, automatic scale 0,
        // rectilinear target and forward correction (reverse=false).
        modifier.Initialize(lens, LF_PF_F32, metadata.focal, metadata.aperture,
                            1000.0f, 0.0f, LF_RECTILINEAR, LF_MODIFY_ALL, false);
        modifier.ApplyColorModification(rgb, 0, 0, width, height, LF_CR_3(RED, GREEN, BLUE),
                                        static_cast<int>(width * 3 * sizeof(float)));
        // A full-frame call preserves Lensfun's accumulated coordinate rounding,
        // which differs measurably from independent row calls on larger images.
        std::vector<float> coordinates(static_cast<size_t>(width) * height * 6);
        const bool geometry = modifier.ApplySubpixelGeometryDistortion(0, 0, width, height, coordinates.data());
        std::vector<float> corrected;
        if (geometry) corrected.resize(static_cast<size_t>(width) * height * 3);
        if (geometry) {
            for (unsigned y = 0; y < height; ++y) {
                for (unsigned x = 0; x < width; ++x) {
                    for (unsigned channel = 0; channel < 3; ++channel) {
                        const size_t coordinate = (static_cast<size_t>(y) * width + x) * 6 + channel * 2;
                        corrected[(static_cast<size_t>(y) * width + x) * 3 + channel] =
                            bilinear(rgb, width, height, channel, coordinates[coordinate], coordinates[coordinate + 1]);
                    }
                }
            }
            std::memcpy(rgb, corrected.data(), corrected.size() * sizeof(float));
        }
        copy_message(summary, summary_len, information.c_str());
        *identity = accepted_identity.release();
        return 0;
    } catch (const std::exception& exception) {
        copy_message(error, error_len, exception.what());
    } catch (...) {
        copy_message(error, error_len, "Unknown native lens correction failure");
    }
    return 1;
}

// The same source-specific selection as correction, without decoding pixels.
// Serialize only the selected camera and lens; unrelated database edits do not
// invalidate an accepted photograph. The caller owns the returned GLib string.
extern "C" int sf_raw_lens_model_identity(const char* path, char** identity,
                                          char* error, size_t error_len) {
    *identity = nullptr;
    copy_message(error, error_len, "");
    try {
        const Metadata metadata = read_metadata(path);
        lfDatabase database;
        const auto [camera, lens] = resolve_model(database, metadata);
        *identity = model_identity(camera, lens);
        return 0;
    } catch (const std::exception& exception) {
        copy_message(error, error_len, exception.what());
        return 1;
    } catch (...) {
        copy_message(error, error_len, "Unknown Lensfun identity failure");
        return 1;
    }
}
extern "C" void sf_raw_free_lens_identity(char* identity) { lf_free(identity); }
// Fixed-size ABI: coefficients only, never coordinates or image samples.
struct SfRawLensParams {
    uint32_t dimensions[4], flags[4];
    float normalization[4], center[4], distortion[4], tca[4], tca_tail[4];
    float vignette[4], mapping[4];
};
static_assert(sizeof(SfRawLensParams) == 144);

namespace {
void split_double(double value, float* parts) {
    parts[0] = static_cast<float>(value);
    parts[1] = static_cast<float>(value - static_cast<double>(parts[0]));
}

uint32_t classify_coordinate_callback(lfDistortionModel model) {
    // Fixed public-API capability probe, independent of the input image/profile.
    // b*r² + (1-b) is exact at these four pairs, including in the PTLens SSE
    // path (a=c=0 removes its approximate radius). Only the scalar/SSE semantic
    // bit escapes; none of these coordinates are used to correct an image.
    lfLens probe;
    probe.Type = LF_RECTILINEAR;
    probe.CropFactor = probe.AspectRatio = 1.0f;
    probe.CenterX = probe.CenterY = 0.0f;
    lfModifier modifier(&probe, 1.0f, 3, 3);
    modifier.AddCoordCallback([](void*, float* coordinates, int count) {
        for (int i = 0; i < count; ++i) {
            coordinates[i * 2] = float(i + 1);
            coordinates[i * 2 + 1] = 0.0f;
        }
    }, 0, nullptr, 0);
    lfLensCalibDistortion calibration{};
    calibration.Model = model;
    calibration.Terms[model == LF_DIST_MODEL_POLY3 ? 0 : 1] = 0.125f;
    if (!modifier.AddCoordCallbackDistortion(calibration, false))
        throw std::runtime_error("Cannot classify Lensfun coordinate callback");
    alignas(16) float coordinates[8]{};
    if (!modifier.ApplyGeometryDistortion(0, 0, 4, 1, coordinates))
        throw std::runtime_error("Cannot classify Lensfun coordinate callback");
    const float scalar[8] = {2.0f, 1.0f, 3.75f, 1.0f, 7.0f, 1.0f, 12.5f, 1.0f};
    const float interleaved[8] = {2.0f, 1.0f, 5.0f, 1.0f, 4.0f, 1.0f, 9.0f, 1.0f};
    if (std::equal(coordinates, coordinates + 8, scalar)) return 0;
    if (std::equal(coordinates, coordinates + 8, interleaved)) return 1;
    throw std::runtime_error("Unsupported Lensfun coordinate callback semantics");
}

uint32_t coordinate_callback_semantics(lfDistortionModel model) {
    if (model == LF_DIST_MODEL_POLY3) {
        static const uint32_t semantics = classify_coordinate_callback(LF_DIST_MODEL_POLY3);
        return semantics;
    }
    if (model == LF_DIST_MODEL_PTLENS) {
        static const uint32_t semantics = classify_coordinate_callback(LF_DIST_MODEL_PTLENS);
        return semantics;
    }
    return 0;
}

// Lensfun 0.3.4's own GetRealFocalLength still consumes legacy CalibFov when
// no real-focal entry exists. InterpolateRealFocal is not equivalent for those
// installed databases. Keep the one compatibility call until that native
// fallback disappears; do not suppress deprecations for the rest of the bridge.
#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wdeprecated-declarations"
#elif defined(_MSC_VER)
#pragma warning(push)
#pragma warning(disable: 4996)
#endif
bool interpolate_legacy_fov(const lfLens& lens, float focal, lfLensCalibFov& result) {
    return lens.InterpolateFov(focal, result);
}
#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic pop
#elif defined(_MSC_VER)
#pragma warning(pop)
#endif

void export_lens(const Metadata& metadata, const lfCamera& camera, const lfLens& lens,
                 unsigned width, unsigned height, int flags, SfRawLensParams& result) {
    // Lensfun 0.3.4 modifier.cpp constructor. Keep float intermediates where
    // upstream has float, and double where its private normalization does.
    const int w = width >= 2 ? width - 1 : 1;
    const int h = height >= 2 ? height - 1 : 1;
    const float size = std::min(w, h);
    const float aspect = float(std::max(w, h)) / float(std::min(w, h));
    const double aspect_correction = std::sqrt(lens.AspectRatio * lens.AspectRatio + 1);
    const float correction = 1.0 / std::sqrt(aspect * aspect + 1) *
                             lens.CropFactor / camera.CropFactor * aspect_correction;
    const double ns = 2.0 / size * correction;
    const double uns = size * 0.5 / correction;
    const double cx = (w / size + lens.CenterX) * correction;
    const double cy = (h / size + lens.CenterY) * correction;
    const double millimeters = std::sqrt(36.0 * 36.0 + 24.0 * 24.0) / 2.0 /
                               aspect_correction / lens.CropFactor;
    result.dimensions[0] = width;
    result.dimensions[1] = height;
    split_double(ns, result.normalization);
    split_double(uns, result.normalization + 2);
    split_double(cx, result.center);
    split_double(cy, result.center + 2);
    result.flags[0] = lens.Type;
    result.flags[1] = bool(flags & LF_MODIFY_GEOMETRY);
    result.flags[2] = bool(flags & LF_MODIFY_VIGNETTING);
    // Select the implementation actually registered by the linked library,
    // rather than guessing from this translation unit's CPU target flags.
    lfLensCalibDistortion distortion;
    if ((flags & LF_MODIFY_DISTORTION) && lens.InterpolateDistortion(metadata.focal, distortion)) {
        result.dimensions[2] = distortion.Model;
        result.flags[3] = coordinate_callback_semantics(distortion.Model);
        std::copy_n(distortion.Terms, 3, result.distortion);
    }
    lfLensCalibTCA tca;
    if ((flags & LF_MODIFY_TCA) && lens.InterpolateTCA(metadata.focal, tca)) {
        result.dimensions[3] = tca.Model;
        std::copy_n(tca.Terms, 4, result.tca);
        std::copy_n(tca.Terms + 4, 2, result.tca_tail);
    }
    lfLensCalibVignetting vignette;
    if ((flags & LF_MODIFY_VIGNETTING) &&
        lens.InterpolateVignetting(metadata.focal, metadata.aperture, 1000.0f, vignette)) {
        std::copy_n(vignette.Terms, 3, result.vignette);
    }
    result.vignette[3] = ns / aspect_correction;
    result.mapping[0] = 1.0 / aspect_correction;
    // GetRealFocalLength, with Initialize's Hugin correction cancelled. Public
    // interpolation APIs retain Lensfun's selection and interpolation rules.
    float focal = metadata.focal;
    lfLensCalibRealFocal real;
    lfLensCalibFov fov;
    const bool has_real_focal = lens.InterpolateRealFocal(metadata.focal, real);
    if (has_real_focal) {
        focal = real.RealFocal;
    } else if (interpolate_legacy_fov(lens, metadata.focal, fov)) {
        const float angle = fov.FieldOfView * 3.14159265358979323846 / 180.0;
        const float half_width = millimeters * lens.AspectRatio;
        switch (lens.Type) {
        case LF_RECTILINEAR: focal = half_width / std::tan(angle / 2.0); break;
        case LF_FISHEYE: case LF_PANORAMIC: case LF_EQUIRECTANGULAR:
            focal = half_width / (angle / 2.0); break;
        case LF_FISHEYE_ORTHOGRAPHIC: focal = half_width / std::sin(angle / 2.0); break;
        case LF_FISHEYE_STEREOGRAPHIC: focal = half_width / (2 * std::tan(angle / 4.0)); break;
        case LF_FISHEYE_EQUISOLID: focal = half_width / (2 * std::sin(angle / 4.0)); break;
        case LF_FISHEYE_THOBY: focal = half_width / (1.47 * std::sin(0.713 * angle / 2.0)); break;
        default: break;
        }
    }
    // Match the two float roundings in GetRealFocalLength / Initialize.
    float hugin = 1.0f;
    if (lens.InterpolateDistortion(metadata.focal, distortion)) {
        if (distortion.Model == LF_DIST_MODEL_POLY3) hugin = 1 - distortion.Terms[0];
        if (distortion.Model == LF_DIST_MODEL_PTLENS)
            hugin = 1 - distortion.Terms[0] - distortion.Terms[1] - distortion.Terms[2];
    }
    focal = (has_real_focal ? focal : static_cast<float>(focal * hugin)) / hugin;
    result.mapping[1] = focal / millimeters;
    result.mapping[2] = 1.0 / result.mapping[1];
    lfModifier unscaled(&lens, camera.CropFactor, width, height);
    unscaled.Initialize(&lens, LF_PF_F32, metadata.focal, metadata.aperture, 1000.0f,
                        1.0f, LF_RECTILINEAR, LF_MODIFY_ALL, false);
    result.mapping[3] = 1.0 / unscaled.GetAutoScale(false);
}
}

extern "C" int sf_raw_prepare_lens(const char* path, unsigned width, unsigned height,
                                    SfRawLensParams* result, int* found,
                                    char* summary, size_t summary_len, char** identity,
                                    char* error, size_t error_len) {
    copy_message(summary, summary_len, "");
    copy_message(error, error_len, "");
    *identity = nullptr;
    try {
        if (!path || !result || !found || !width || !height ||
            width > unsigned(std::numeric_limits<int>::max()) ||
            height > unsigned(std::numeric_limits<int>::max()))
            throw std::invalid_argument("Lens preparation requires a path and nonempty native dimensions");
        *result = {};
        *found = 0;
        const Metadata metadata = read_metadata(path);
        lfDatabase database;
        const auto [camera, lens] = resolve_model(database, metadata);
        std::unique_ptr<char, GlibFree> accepted_identity(model_identity(camera, lens));
        if (lens) {
            lfModifier modifier(lens, camera->CropFactor, static_cast<int>(width), static_cast<int>(height));
            // The same initialization as CPU correction; only coefficients leave this route.
            const int flags = modifier.Initialize(lens, LF_PF_F32, metadata.focal, metadata.aperture,
                                                 1000.0f, 0.0f, LF_RECTILINEAR, LF_MODIFY_ALL, false);
            export_lens(metadata, *camera, *lens, width, height, flags, *result);
            const std::string label = lens->Model && *lens->Model ? lens->Model : metadata.lens_model;
            const std::string information = label + " @ " + float_label(metadata.focal) +
                                            "mm f/" + float_label(metadata.aperture);
            copy_message(summary, summary_len, information.c_str());
            *found = 1;
        }
        *identity = accepted_identity.release();
        return 0;
    } catch (const std::exception& exception) {
        copy_message(error, error_len, exception.what());
    } catch (...) {
        copy_message(error, error_len, "Unknown native lens preparation failure");
    }
    return 1;
}
