// G4: independent and layer-shared conditional Poisson masses.
// The count law and address functions are shared verbatim with certification.
const WORKGROUP: u32 = 256u;

struct AccumulateParams {
    geometry: vec4<i32>, // global extended origin, extended width and height
    seed: vec2<u32>,
    cell_area: f32,
    correlation: f32,
    expectation: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var<uniform> params: AccumulateParams;
@group(0) @binding(1) var<storage, read> tables: array<Table>;
@group(0) @binding(2) var<storage, read> layers: array<f32>;
@group(0) @binding(3) var<storage, read_write> cells: array<f32>;
@group(0) @binding(4) var<storage, read_write> status: atomic<u32>;

fn draw(lam: f32, i: i32, j: i32, tag: u32) -> f32 {
    let sample = count_of(lam, address(i, j, tag, params.seed.x, params.seed.y));
    if (sample.x == 0xFFFFFFFFu) {
        atomicOr(&status, 4u);
        return 0.0;
    }
    return f32(sample.x);
}

@compute @workgroup_size(WORKGROUP)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = gid.x + gid.y * grid.x * WORKGROUP;
    let cols = u32(params.geometry.z);
    if (index >= cols * u32(params.geometry.w)) { return; }
    let i = params.geometry.x + i32(index % cols);
    let j = params.geometry.y + i32(index / cols);
    var result = vec3<f32>(0.0);
    for (var layer = 0u; layer < 3u; layer = layer + 1u) {
        var rate = vec3<f32>(0.0);
        var mass = vec3<f32>(0.0);
        var enabled = vec3<bool>(false);
        var smallest = 3.402823466e38;
        var active_channels = 0u;
        for (var channel = 0u; channel < 3u; channel = channel + 1u) {
            let table = tables[channel];
            let value = layers[index * 9u + channel * 3u + layer];
            if (value <= 0.0 || table.cap[layer] <= 0.0) { continue; }
            let occupancy = clamp(value / table.cap[layer], 0.0, 1.0);
            // Algebraically 1-u*(1-eps)*p, without cancellation at u=p=1.
            let s = (1.0 - table.uniformity) + table.uniformity *
                ((1.0 - occupancy) + 1e-6 * occupancy);
            mass[channel] = table.cap[layer] * table.area[layer] * s / table.frac[layer];
            rate[channel] = params.cell_area * table.frac[layer] * occupancy / (table.area[layer] * s);
            enabled[channel] = true;
            if (!(rate[channel] > 0.0 && rate[channel] <= MAX_RATE && mass[channel] > 0.0 && mass[channel] <= 3.402823466e38)) {
                atomicOr(&status, 2u);
                return;
            }
            active_channels += 1u;
            smallest = min(smallest, rate[channel]);
        }
        var shared_rate = 0.0;
        if (active_channels >= 2u) { shared_rate = params.correlation * smallest; }
        var shared_count = 0.0;
        if (params.expectation == 0u && shared_rate > 0.0) {
            shared_count = draw(shared_rate, i, j, 9u + layer);
        }
        for (var channel = 0u; channel < 3u; channel = channel + 1u) {
            if (!enabled[channel]) { continue; }
            var count = rate[channel];
            if (params.expectation == 0u) {
                let independent = rate[channel] - shared_rate;
                count = shared_count;
                if (independent > 0.0) { count += draw(independent, i, j, channel * 3u + layer); }
            }
            result[channel] += mass[channel] * count / params.cell_area;
        }
    }
    if (!all(result >= vec3<f32>(0.0)) || !all(result <= vec3<f32>(3.402823466e38))) {
        atomicOr(&status, 2u);
    }
    cells[index * 3u] = result.x;
    cells[index * 3u + 1u] = result.y;
    cells[index * 3u + 2u] = result.z;
}
