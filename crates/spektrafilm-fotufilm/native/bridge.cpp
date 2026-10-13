#include "bridge.h"
#include "FotufilmHalide.h"
#include "FotufilmTransport.h"
#include "FotufilmFilmTileBuild.h"
#include "Pipeline/Transport.h"
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <chrono>
#include <exception>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <vector>

extern "C" float sf_fotufilm_expf(float x) { return ::expf(x); }
extern "C" float sf_fotufilm_exp2f(float x) { return ::exp2f(x); }
extern "C" float sf_fotufilm_logf(float x) { return ::logf(x); }
extern "C" float sf_fotufilm_log1pf(float x) { return ::log1pf(x); }
extern "C" float sf_fotufilm_log2f(float x) { return ::log2f(x); }
extern "C" float sf_fotufilm_log10f(float x) { return ::log10f(x); }
extern "C" float sf_fotufilm_powf(float x, float y) { return ::powf(x, y); }
extern "C" float sf_fotufilm_tanf(float x) { return ::tanf(x); }
extern "C" int32_t sf_fotufilm_film_tile_build(int32_t texels, int32_t supersample,
    int32_t mark_shape, const float *terms, int32_t term_count, const float *sublayers,
    const float *fractions, int32_t levels, uint32_t seed, int32_t record, float *light) {
    return fotufilm_film_tile_build(texels, supersample, mark_shape, terms, term_count,
        sublayers, fractions, levels, seed, record, light);
}

namespace {
constexpr size_t lut_count = 33 * 33 * 33 * 4;
constexpr size_t bank_count = size_t(FOTUFILM_FILM_TILE_SIDE + 1)
    * (FOTUFILM_FILM_TILE_SIDE + 1) * FOTUFILM_FILM_TILE_LEVELS * 3;
std::timed_mutex process_mutex;
uint32_t next_bank = 1;
struct Failure { int32_t status; };
void checked(int32_t status) { if (status) throw Failure{status}; }
struct Cancellation {
    sf_fotufilm_cancel callback; void *context;
    void check() const { if (callback && callback(context)) throw Failure{1}; }
};
struct Invocation {
    sf_fotufilm_invocation args;
    std::vector<float> configuration;
    Invocation(const sf_fotufilm_invocation &a, int32_t bank) : args(a) {
        if (!a.configuration || a.configuration_count != FOTUFILM_FRAME_CONFIGURATION_COUNT
            || !a.exposure || !a.film || a.lut_float_count != lut_count
            || a.lut_dimension != 33 || a.spatial_support < 0)
            throw std::invalid_argument("invalid original configuration/table geometry");
        configuration.assign(a.configuration, a.configuration + a.configuration_count);
        configuration[FOTUFILM_CONFIG_FILM_TILE + 2] = float(bank);
        args.configuration = configuration.data();
        if (!args.paper) args.paper = args.film;
    }
};
struct Bank {
    int32_t id = 0;
    Bank(const float *tiles, size_t count, bool required) {
        if (required && (!tiles || count != bank_count))
            throw std::invalid_argument("Film grain requires its complete original running-sum bank");
        if (count && (!tiles || count != bank_count))
            throw std::invalid_argument("invalid original Film running-sum bank size");
        if (count) {
            // Calls serialize registration and retirement; every allocated identity is exactly
            // representable in binary32 and is never reused, even after its bank is retired.
            if (next_bank > (1u << 24)) throw std::runtime_error("Film bank identity space exhausted");
            id = int32_t(next_bank++);
            checked(fotufilm_halide_set_film_tiles(id, tiles, int64_t(count)));
        }
    }
    ~Bank() { if (id) fotufilm_halide_set_film_tiles(id, nullptr, 0); }
};
void render(const float *const input[3], float *const output[3], int32_t w, int32_t h,
            Invocation &inv, const Cancellation &cancel, const float *donor = nullptr,
            bool light = false) {
    // Preserve the original spatial support, full photograph dimensions, and absolute origin.
    // Full-width strips also retain the original decimation-cell alignment.
    const int32_t apron = std::max(1, inv.args.spatial_support);
    // An unmeasured original CPU flare head reduces its complete post-lens light.
    // It cannot measure a strip or reuse another component's spectral mean.
    const bool full_frame_flare = (inv.args.feature_mask & FOTUFILM_FRAME_FLARE)
        && inv.configuration[FOTUFILM_CONFIG_FLARE_MEAN] < 0;
    for (int32_t top = 0; top < h; ) {
        cancel.check();
        const int32_t bottom = full_frame_flare ? h : top + std::min<int32_t>(64, h - top);
        const int32_t from = int32_t(std::max<int64_t>(0, int64_t(top) - apron));
        const int32_t to = int32_t(std::min<int64_t>(h, int64_t(bottom) + apron));
        std::vector<float> extra;
        if (donor) {
            extra.resize(size_t(w) * (to - from) * 4, 0);
            for (size_t i = 0; i < size_t(w) * (to - from); ++i)
                extra[4 * i + 3] = donor[size_t(from) * w + i];
        }
        const int32_t mask = inv.args.feature_mask
            | (light ? FOTUFILM_FRAME_DENSITY_OUT : 0);
        checked(fotufilm_halide_process_tile_with_exposure(
            input[0] + size_t(from) * w, input[1] + size_t(from) * w,
            input[2] + size_t(from) * w, output[0], output[1], output[2],
            w, to - from, w, h, 0, from, 0, top - from, w, bottom - top,
            inv.configuration.data(), inv.args.exposure, inv.args.film, inv.args.paper,
            inv.args.lut_dimension, mask, inv.args.seed, donor ? extra.data() : nullptr));
        top = bottom;
    }
    cancel.check();
}
void clear_optics(Invocation &a, bool keep_lens) {
    auto &v = a.configuration;
    for (int c = 0; c < 3; ++c) {
        v[FOTUFILM_CONFIG_MTF_RADIUS + c] = 0;
        v[FOTUFILM_CONFIG_MTF_SECONDARY_RADIUS + c] = 0;
        v[FOTUFILM_CONFIG_HALATION_RADIUS + c] = 0;
    }
    v[FOTUFILM_CONFIG_MTF_LUMA_RADIUS] = 0;
    v[FOTUFILM_CONFIG_MTF_LUMA_SHARE] = 0;
    for (int c = 0; c < 9; ++c) v[FOTUFILM_CONFIG_HALATION_MATRIX + c] = 0;
    if (!keep_lens) {
        v[FOTUFILM_CONFIG_GATE + 4] = -1;
        v[FOTUFILM_CONFIG_CAMERA_PREFLASH] = 0;
        v[FOTUFILM_CONFIG_FLARE] = 0;
        v[FOTUFILM_CONFIG_DIFFUSION_DIRECT] = 1;
        for (int c = 0; c < 9; ++c) v[FOTUFILM_CONFIG_DIFFUSION_KERNEL + c] = 0;
        for (int c = 0; c < 3; ++c) v[FOTUFILM_CONFIG_DIFFUSION_RADIUS + c] = 0;
    }
}
void donor_head(Invocation &head, std::vector<float> &table) {
    table.assign(head.args.exposure, head.args.exposure + lut_count);
    for (size_t i = 0; i < lut_count; i += 4)
        table[i] = table[i + 1] = table[i + 2] = table[i + 3];
    head.args.exposure = table.data();
    for (int c = 0; c < 3; ++c) for (int k = 0; k < 3; ++k)
        head.configuration[FOTUFILM_CONFIG_DIFFUSION_KERNEL + 3*c+k]
            = head.configuration[FOTUFILM_CONFIG_DONOR_DIFFUSION_KERNEL + k];
}
// Original TransportPipeline, with full-frame coordinates and bounded output regions.
void transport(const float *exposure, float *sum, int32_t w, int32_t h, int32_t channels,
               const float *stencils, const Cancellation &cancel,
               const Invocation *head = nullptr, bool first = false) {
    using namespace Halide;
    using fotufilm::pipelines::TransportPipeline;
    if (head) checked(fotufilm_transport_validate_table(stencils));
    else checked(fotufilm_transport_validate(exposure, sum, w, h, channels, stencils, 0));
    static TransportPipeline *pipelines[2] = {new TransportPipeline(DeviceAPI::None, false),
                                            new TransportPipeline(DeviceAPI::None, true)};
    auto *pipeline = pipelines[head != nullptr];
    const auto target = get_host_target().with_feature(Target::StrictFloat);
    halide_dimension_t domain_shape[3] = {{0,w,4},{0,h,4*w},{0,4,1}};
    Buffer<float> input = head ? Buffer<float>(const_cast<float *>(exposure), 3, domain_shape)
                              : Buffer<float>(const_cast<float *>(exposure), w, h, channels);
    Buffer<float> accumulated(sum, w, h, channels);
    Buffer<float> table(const_cast<float *>(stencils), FOTUFILM_TRANSPORT_TABLE_FLOATS);
    input.set_host_dirty(); accumulated.set_host_dirty(); table.set_host_dirty();
    pipeline->exposure.set(input);
    pipeline->accumulated.set(accumulated);
    pipeline->stencils.set(table);
    if (head) {
        Buffer<float> config(const_cast<float *>(head->configuration.data()), FOTUFILM_FRAME_CONFIGURATION_COUNT);
        Buffer<float> lut(const_cast<float *>(head->args.exposure), int(lut_count));
        config.set_host_dirty(); lut.set_host_dirty();
        pipeline->configuration.set(config); pipeline->exposure_lut.set(lut); pipeline->first.set(first);
    }
    struct Unbind { TransportPipeline *p; ~Unbind() { p->exposure.reset(); p->accumulated.reset(); p->stencils.reset(); p->configuration.reset(); p->exposure_lut.reset(); } } unbind{pipeline};
    for (int l = 0; l < FOTUFILM_TRANSPORT_LEVELS; ++l) pipeline->radii[l].set(int32_t(stencils[l]));
    for (int32_t top = 0; top < h; ) {
        cancel.check();
        const int32_t rows = std::min<int32_t>(64, h - top);
        halide_dimension_t shape[3] = {{0,w,1},{top,rows,w},{0,channels,w*h}};
        Buffer<float> region(sum + size_t(top)*w, 3, shape);
        pipeline->output.realize(region, target);
        checked(region.copy_to_host());
        top += rows;
    }
}
}

extern "C" size_t sf_fotufilm_configuration_count() { return FOTUFILM_FRAME_CONFIGURATION_COUNT; }
extern "C" size_t sf_fotufilm_lut_float_count() { return lut_count; }
extern "C" size_t sf_fotufilm_tile_float_count() { return bank_count; }
extern "C" const char *sf_fotufilm_source_revision() { return "59d43471dbef45c85ae442e507a291fdebfb2ec5"; }
extern "C" int32_t sf_fotufilm_process(
    const sf_fotufilm_invocation *base, const sf_fotufilm_invocation *tail,
    const sf_fotufilm_component *components, size_t component_count, int32_t channels,
    const float *tiles, size_t tile_count,
    const float *r, const float *g, const float *b, float *rr, float *gg, float *bb,
    int32_t w, int32_t h, sf_fotufilm_cancel callback, void *context,
    char *error, size_t error_capacity) {
    auto fail = [&](int32_t status, const char *message) {
        if (error && error_capacity) std::snprintf(error, error_capacity, "%s", message);
        return status;
    };
    if (error && error_capacity) error[0] = 0;
    try {
        if (!base || !r || !g || !b || !rr || !gg || !bb || w < 1 || h < 1
            || int64_t(w)*h > 150000000 || channels < 3 || channels > 4
            || (component_count && (!components || !tail)))
            throw std::invalid_argument("invalid native photograph/transport buffers");
        Cancellation cancel{callback, context};
        cancel.check();
        std::unique_lock<std::timed_mutex> lock(process_mutex, std::defer_lock);
        while (!lock.try_lock_for(std::chrono::milliseconds(20))) cancel.check();
        cancel.check();
        if (!base->configuration || base->configuration_count != FOTUFILM_FRAME_CONFIGURATION_COUNT)
            throw std::invalid_argument("invalid source configuration length");
        Bank bank(tiles, tile_count, base->configuration[FOTUFILM_CONFIG_GRAIN_MODE] == 1);
        Invocation ordinary(*base, bank.id);
        const float *input[3] = {r,g,b}; float *output[3] = {rr,gg,bb};
        if (!tail) { render(input, output, w, h, ordinary, cancel); return 0; }
        Invocation continuation(*tail, bank.id);
        continuation.args.spatial_support = std::max(base->spatial_support, tail->spatial_support);
        const size_t n = size_t(w)*h;
        std::vector<float> sum(n*channels, 0), component(n*channels), fourth(channels == 4 ? n*3 : 0);
        float *component_planes[3] = {component.data(), component.data()+n, component.data()+2*n};
        const bool scene_heads = !(base->feature_mask & (FOTUFILM_FRAME_FLARE | FOTUFILM_FRAME_DIFFUSION));
        std::vector<float> domain;
        if (scene_heads && component_count) {
            using namespace Halide;
            static auto *domain_pipeline = new fotufilm::pipelines::TransportDomainPipeline(DeviceAPI::None);
            std::vector<float> scene(n*4, 1);
            for (size_t i=0;i<n;++i) { scene[4*i]=r[i]; scene[4*i+1]=g[i]; scene[4*i+2]=b[i]; }
            domain.resize(n*4);
            halide_dimension_t shape[3] = {{0,w,4},{0,h,4*w},{0,4,1}};
            Buffer<float> source(scene.data(),3,shape);
            Invocation first_head(components[0].head,bank.id);
            Buffer<float> config(first_head.configuration.data(),FOTUFILM_FRAME_CONFIGURATION_COUNT);
            source.set_host_dirty(); config.set_host_dirty();
            domain_pipeline->scene.set(source); domain_pipeline->configuration.set(config);
            struct UnbindDomain { fotufilm::pipelines::TransportDomainPipeline *p;
                ~UnbindDomain() { p->scene.reset(); p->configuration.reset(); } } unbind{domain_pipeline};
            const auto target=get_host_target().with_feature(Target::StrictFloat);
            for(int32_t top=0;top<h;) {
                cancel.check();
                const int32_t rows=std::min<int32_t>(64,h-top);
                halide_dimension_t region_shape[3]={{0,w,4},{top,rows,4*w},{0,4,1}};
                Buffer<float> region(domain.data()+size_t(top)*w*4,3,region_shape);
                domain_pipeline->output.realize(region,target);
                checked(region.copy_to_host()); top+=rows;
            }
        }
        // Preserve component order and original donor light-head order, including donor diffusion.
        for (size_t k = 0; k < component_count; ++k) {
            cancel.check();
            if (components[k].stencil_count != FOTUFILM_TRANSPORT_TABLE_FLOATS)
                throw std::invalid_argument("invalid original transport stencil count");
            Invocation head(components[k].head, bank.id);
            head.args.spatial_support = std::max(head.args.spatial_support, base->spatial_support);
            if (scene_heads) {
                transport(domain.data(),sum.data(),w,h,channels,components[k].stencils,cancel,&head,k==0);
                continue;
            }
            render(input, component_planes, w, h, head, cancel, nullptr, true);
            if (channels == 4) {
                std::vector<float> donor_table;
                donor_head(head, donor_table);
                float *donor_planes[3] = {fourth.data(),fourth.data()+n,fourth.data()+2*n};
                render(input, donor_planes, w, h, head, cancel, nullptr, true);
                std::copy_n(fourth.data(), n, component.data()+3*n);
            }
            transport(component.data(), sum.data(), w, h, channels, components[k].stencils, cancel);
        }
        for (float value : sum) if (!std::isfinite(value) || value < 0)
            throw std::runtime_error("transport produced invalid record exposure");
        const float *transported[3] = {sum.data(),sum.data()+n,sum.data()+2*n};
        const float *donor = channels == 4 ? sum.data()+3*n : nullptr;
        const bool texture = (base->feature_mask & FOTUFILM_FRAME_TEXTURE) != 0;
        if (texture) continuation.args.feature_mask |= FOTUFILM_FRAME_DENSITY_OUT;
        render(transported, output, w, h, continuation, cancel, donor);
        if (texture) {
            Invocation reference_head(*base, bank.id);
            reference_head.args.feature_mask &= FOTUFILM_FRAME_FLARE | FOTUFILM_FRAME_DIFFUSION;
            reference_head.args.feature_mask |= FOTUFILM_FRAME_LIGHT_OUT;
            clear_optics(reference_head, true);
            render(input, component_planes, w, h, reference_head, cancel, nullptr, true);
            if (channels == 4) {
                std::vector<float> donor_table;
                donor_head(reference_head, donor_table);
                float *donor_planes[3] = {fourth.data(),fourth.data()+n,fourth.data()+2*n};
                render(input, donor_planes, w, h, reference_head, cancel, nullptr, true);
                std::copy_n(fourth.data(),n,component.data()+3*n);
            }
            Invocation reference(*tail, bank.id);
            reference.args.spatial_support = continuation.args.spatial_support;
            reference.args.feature_mask |= FOTUFILM_FRAME_DENSITY_OUT;
            reference.args.feature_mask &= ~(FOTUFILM_FRAME_GRAIN | FOTUFILM_FRAME_ADJACENCY
                | FOTUFILM_FRAME_COUPLER_DIFFUSION | FOTUFILM_FRAME_PRINT_MTF);
            reference.configuration[FOTUFILM_CONFIG_CHROMATIC_FRINGE_AMOUNT] = 0;
            reference.configuration[FOTUFILM_CONFIG_PRINT_MTF_RADIUS] = 0;
            reference.configuration[FOTUFILM_CONFIG_GRAIN_RADIUS] = 0;
            reference.configuration[FOTUFILM_CONFIG_ADJACENCY_STRENGTH] = 0;
            reference.configuration[FOTUFILM_CONFIG_COUPLER_RADIUS] = 0;
            for (int c=0;c<3;++c) reference.configuration[FOTUFILM_CONFIG_GRAIN + c] = 0;
            std::vector<float> baseline(n*3);
            float *baseline_planes[3] = {baseline.data(),baseline.data()+n,baseline.data()+2*n};
            const float *reference_planes[3] = {component.data(),component.data()+n,component.data()+2*n};
            render(reference_planes, baseline_planes, w,h,reference,cancel,channels==4?component.data()+3*n:nullptr);
            const float sign = ordinary.configuration[FOTUFILM_CONFIG_DEVELOP_COMPLEMENT] != 0 ? -1.0f : 1.0f;
            for(int c=0;c<3;++c) for(size_t i=0;i<n;++i)
                output[c][i] = input[c][i] * std::exp(sign * (output[c][i]-baseline_planes[c][i]) * std::log(10.0f));
        }
        cancel.check();
        return 0;
    } catch (const Failure &failure) {
        return fail(failure.status, failure.status == 1 ? "Fotufilm rendering canceled" : "original Fotufilm native source rejected rendering");
    } catch (const Halide::Error &exception) { return fail(-1, exception.what());
    } catch (const std::exception &exception) { return fail(-1, exception.what()); }
      catch (...) { return fail(-2, "unknown original Fotufilm native error"); }
}
