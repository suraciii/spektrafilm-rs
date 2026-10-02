#include <exiv2/exiv2.hpp>
#include <glib.h>
#include <lensfun.h>

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
}

extern "C" int sf_raw_correct_lens(const char* path, float* rgb, unsigned width, unsigned height,
                                    char* summary, size_t summary_len, char* error, size_t error_len) {
    copy_message(summary, summary_len, "");
    copy_message(error, error_len, "");
    try {
        if (!path || !rgb || width == 0 || height == 0 ||
            width > static_cast<unsigned>(std::numeric_limits<int>::max() / (3 * sizeof(float))) ||
            height > static_cast<unsigned>(std::numeric_limits<int>::max()) ||
            static_cast<size_t>(height) > std::numeric_limits<size_t>::max() / width / 6 / sizeof(float)) {
            throw std::invalid_argument("Lens correction requires a path and a nonempty float32 RGB image within native size limits");
        }
        const Metadata metadata = read_metadata(path);
        if (metadata.lens_model.empty()) return 0;
        lfDatabase database;
        const lfError status = load_database(database);
        if (status != LF_NO_ERROR) {
            throw std::runtime_error("Cannot load the Lensfun database (error " + std::to_string(status) +
                                     "). Install the Lensfun lens database package or update it with lensfun-update-data.");
        }
        std::unique_ptr<const lfCamera*, GlibFree> cameras(
            database.FindCamerasExt(metadata.make.c_str(), metadata.model.c_str(), LF_SEARCH_LOOSE));
        if (!cameras || !*cameras) return 0;
        const lfCamera* camera = *cameras;
        const auto candidates = find_candidates(database, camera, metadata);
        if (candidates.empty()) return 0;
        const lfLens* lens = select_candidate(candidates, metadata);
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
        return 0;
    } catch (const std::exception& exception) {
        copy_message(error, error_len, exception.what());
    } catch (...) {
        copy_message(error, error_len, "Unknown native lens correction failure");
    }
    return 1;
}
