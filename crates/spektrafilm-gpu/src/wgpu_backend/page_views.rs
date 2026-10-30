//! Checked common page intersections for pointwise storage bindings.
use super::grain::RasterStorage;
use super::*;

pub(super) const ADAPTER_WORKSPACE_BYTES: u64 = 8 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Access {
    Read,
    Write,
    ReadWrite,
    /// A dispatch owns only some samples; preserve all other samples.
    WritePartial,
    Fixed,
}
impl Access {
    fn reads(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite | Self::WritePartial)
    }
    fn writes(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite | Self::WritePartial)
    }
}

#[derive(Clone)]
pub(super) struct StorageView {
    pub buffer: wgpu::Buffer,
    pub offset: u64,
    pub size: u64,
    /// Scalar f32/u32 residual, never a pixel residual.
    pub base: u32,
}
impl StorageView {
    pub fn binding(&self) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.buffer,
            offset: self.offset,
            size: std::num::NonZeroU64::new(self.size),
        })
    }
    pub fn entire(buffer: &wgpu::Buffer) -> Self {
        Self {
            buffer: buffer.clone(),
            offset: 0,
            size: buffer.size(),
            base: 0,
        }
    }
}
struct Page {
    first: u64,
    bytes: u64,
    buffer: wgpu::Buffer,
}
struct Binding {
    pages: Vec<Page>,
    access: Access,
    scratch: Option<wgpu::Buffer>,
}
impl Binding {
    fn page_at(&self, position: u64) -> Option<&Page> {
        let index = self
            .pages
            .partition_point(|page| page.first <= position)
            .checked_sub(1)?;
        self.pages
            .get(index)
            .filter(|page| position < page.first + page.bytes)
    }
}
pub(super) struct ViewChunk {
    pub first: u64,
    pub count: u32,
    pub views: Vec<StorageView>,
}
pub(super) struct ViewPlan {
    bindings: Vec<Binding>,
    n: u64,
    bytes_per_item: u64,
    capacity: u64,
    alignment: u64,
    limit: u64,
    label: String,
}
impl ViewPlan {
    pub fn new(
        backend: &WgpuBackend,
        n: u64,
        bytes_per_item: u64,
        buffers: &[(&dyn RasterStorage, Access)],
        label: &str,
    ) -> Result<Self, String> {
        let limit = super::bounded::storage_binding_limit(backend)?;
        let alignment =
            u64::from(backend.device.limits().min_storage_buffer_offset_alignment).max(4);
        if n == 0
            || n > u64::from(u32::MAX)
            || bytes_per_item == 0
            || !bytes_per_item.is_multiple_of(4)
        {
            return Err(format!("{label}: invalid pointwise item geometry"));
        }
        let bytes = n
            .checked_mul(bytes_per_item)
            .ok_or_else(|| format!("{label}: image size overflow"))?;
        let scratch_capacity = 65_536u64
            .min(limit / bytes_per_item)
            .min((ADAPTER_WORKSPACE_BYTES / buffers.len().max(1) as u64) / bytes_per_item)
            .min(n);
        if scratch_capacity == 0 {
            return Err(format!("{label}: pointwise workspace cannot hold one item"));
        }
        let mut bindings = Vec::new();
        for (raster, access) in buffers {
            let mut pages = Vec::new();
            let mut end = 0;
            for page in raster.pages() {
                let first = u64::from(page.first_pixel) * 12;
                let size = if *access == Access::Fixed {
                    page.buffer.size()
                } else {
                    (u64::from(page.n_pixels) * 12).min(page.buffer.size())
                };
                if size == 0 || first != end {
                    return Err(format!(
                        "{label}: page table must be ordered, nonempty and cover without gaps or overlaps"
                    ));
                }
                end = first
                    .checked_add(size)
                    .ok_or_else(|| format!("{label}: page interval overflow"))?;
                pages.push(Page {
                    first,
                    bytes: size,
                    buffer: page.buffer,
                });
            }
            if pages.is_empty() || (*access != Access::Fixed && end < bytes) {
                return Err(format!(
                    "{label}: pointwise raster is shorter than its logical image"
                ));
            }
            if *access == Access::Fixed
                && (pages.len() != 1
                    || pages[0].bytes > limit
                    || !pages[0]
                        .buffer
                        .usage()
                        .contains(wgpu::BufferUsages::STORAGE))
            {
                return Err(format!(
                    "{label}: fixed coefficients must fit one STORAGE binding"
                ));
            }
            bindings.push(Binding {
                pages,
                access: *access,
                scratch: None,
            });
        }
        let mut adapted = vec![false; bindings.len()];
        struct Owners {
            bindings: Vec<(usize, u64, Access)>,
            writer: bool,
        }
        // wgpu Buffer hashes its immutable resource identity, not mapped contents.
        #[allow(clippy::mutable_key_type)]
        let mut physical: std::collections::HashMap<&wgpu::Buffer, Owners> =
            std::collections::HashMap::new();
        for (index, binding) in bindings.iter().enumerate() {
            for page in &binding.pages {
                let owners = physical.entry(&page.buffer).or_insert_with(|| Owners {
                    bindings: Vec::new(),
                    writer: false,
                });
                if binding.access.writes() || owners.writer {
                    for &(other, first, access) in &owners.bindings {
                        if !binding.access.writes() && !access.writes() {
                            continue;
                        }
                        if page.first != first
                            || binding.access == Access::Fixed
                            || access == Access::Fixed
                            || (index != other && binding.access.writes() && access.writes())
                        {
                            return Err(format!(
                                "{label}: cross-chunk physical alias would overwrite unread samples"
                            ));
                        }
                        adapted[index] = true;
                        adapted[other] = true;
                    }
                }
                owners.writer |= binding.access.writes();
                owners.bindings.push((index, page.first, binding.access));
            }
        }
        drop(physical);
        for (index, binding) in bindings.iter_mut().enumerate() {
            if binding.access == Access::Fixed {
                continue;
            }
            adapted[index] |= binding.pages.iter().any(|page| {
                !page.buffer.usage().contains(wgpu::BufferUsages::STORAGE)
                    || (page.bytes > limit && limit < alignment + bytes_per_item - 4)
            });
            if adapted[index] {
                let mut required = wgpu::BufferUsages::empty();
                if binding.access.reads() {
                    required |= wgpu::BufferUsages::COPY_SRC;
                }
                if binding.access.writes() {
                    required |= wgpu::BufferUsages::COPY_DST;
                }
                if binding
                    .pages
                    .iter()
                    .any(|page| !page.buffer.usage().contains(required))
                {
                    return Err(format!("{label}: fallback raster is missing {required:?}"));
                }
                binding.scratch = Some(backend.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: scratch_capacity * bytes_per_item,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
        }
        // Validate all intersections before submitting even the first write.
        let capacity = if bindings.iter().any(|binding| binding.scratch.is_some()) {
            scratch_capacity
        } else {
            (limit / bytes_per_item).min(n)
        };
        let plan = Self {
            bindings,
            n,
            bytes_per_item,
            capacity,
            alignment,
            limit,
            label: label.into(),
        };
        let mut first = 0;
        while first < n {
            first += u64::from(plan.chunk(first)?.count);
        }
        Ok(plan)
    }
    fn chunk(&self, first: u64) -> Result<ViewChunk, String> {
        let position = first * self.bytes_per_item;
        let mut count = self.capacity.min(self.n - first);
        for binding in &self.bindings {
            if binding.access == Access::Fixed || binding.scratch.is_some() {
                continue;
            }
            let page = binding
                .page_at(position)
                .ok_or_else(|| format!("{}: uncovered pointwise item", self.label))?;
            let local = position - page.first;
            let residual = local % self.alignment;
            count = count
                .min((page.bytes - local) / self.bytes_per_item)
                .min(self.limit.saturating_sub(residual) / self.bytes_per_item);
        }
        if count == 0 {
            return Err(format!(
                "{}: page seam splits a pointwise item or exceeds binding limits",
                self.label
            ));
        }
        let mut views = Vec::with_capacity(self.bindings.len());
        for binding in &self.bindings {
            let view = if let Some(scratch) = &binding.scratch {
                StorageView {
                    buffer: scratch.clone(),
                    offset: 0,
                    size: count * self.bytes_per_item,
                    base: 0,
                }
            } else if binding.access == Access::Fixed {
                StorageView {
                    buffer: binding.pages[0].buffer.clone(),
                    offset: 0,
                    size: binding.pages[0].bytes,
                    base: 0,
                }
            } else {
                let page = binding.page_at(position).unwrap();
                let local = position - page.first;
                let offset = local / self.alignment * self.alignment;
                StorageView {
                    buffer: page.buffer.clone(),
                    offset,
                    size: local - offset + count * self.bytes_per_item,
                    base: u32::try_from((local - offset) / 4)
                        .map_err(|_| format!("{}: residual overflow", self.label))?,
                }
            };
            views.push(view);
        }
        Ok(ViewChunk {
            first,
            count: count as u32,
            views,
        })
    }
    fn copies(&self, encoder: &mut ObservedEncoder, chunk: &ViewChunk, scatter: bool) {
        let first = chunk.first * self.bytes_per_item;
        let end = first + u64::from(chunk.count) * self.bytes_per_item;
        for binding in &self.bindings {
            let Some(scratch) = &binding.scratch else {
                continue;
            };
            if !(if scatter {
                binding.access.writes()
            } else {
                binding.access.reads()
            }) {
                continue;
            }
            for page in &binding.pages {
                let low = first.max(page.first);
                let high = end.min(page.first + page.bytes);
                if low >= high {
                    continue;
                }
                if scatter {
                    encoder.copy_buffer_to_buffer(
                        scratch,
                        low - first,
                        &page.buffer,
                        low - page.first,
                        high - low,
                    );
                } else {
                    encoder.copy_buffer_to_buffer(
                        &page.buffer,
                        low - page.first,
                        scratch,
                        low - first,
                        high - low,
                    );
                }
            }
        }
    }
    pub fn for_each(
        &self,
        backend: &WgpuBackend,
        mut execute: impl FnMut(&mut ObservedEncoder, &ViewChunk) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut first = 0;
        while first < self.n {
            backend.check_cancelled()?;
            let chunk = self.chunk(first)?;
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            self.copies(&mut encoder, &chunk, false);
            execute(&mut encoder, &chunk)?;
            self.copies(&mut encoder, &chunk, true);
            backend.check_cancelled()?;
            backend.submit_commands(Some(encoder.finish()))?;
            first += u64::from(chunk.count);
        }
        backend.check_cancelled()
    }
    /// Kernels such as spectral LUTs submit several sparse brick dispatches
    /// before the output may be scattered. Keep one view alive for all bricks.
    pub fn for_each_submitted(
        &self,
        backend: &WgpuBackend,
        mut execute: impl FnMut(&ViewChunk) -> Result<(), String>,
    ) -> Result<(), String> {
        let gather = self
            .bindings
            .iter()
            .any(|b| b.scratch.is_some() && b.access.reads());
        let scatter = self
            .bindings
            .iter()
            .any(|b| b.scratch.is_some() && b.access.writes());
        let mut first = 0;
        while first < self.n {
            backend.check_cancelled()?;
            let chunk = self.chunk(first)?;
            if gather {
                let mut encoder = backend.device.create_command_encoder(&Default::default());
                self.copies(&mut encoder, &chunk, false);
                backend.check_cancelled()?;
                backend.submit_commands(Some(encoder.finish()))?;
            }
            execute(&chunk)?;
            backend.check_cancelled()?;
            if scatter {
                let mut encoder = backend.device.create_command_encoder(&Default::default());
                self.copies(&mut encoder, &chunk, true);
                backend.check_cancelled()?;
                backend.submit_commands(Some(encoder.finish()))?;
            }
            first += u64::from(chunk.count);
        }
        backend.check_cancelled()
    }
}

impl WgpuBackend {
    /// Address-only specialization; shader arithmetic and expression order stay unchanged.
    pub(super) fn cached_pointwise_pipeline(
        &self,
        shader: &'static str,
        types: &[wgpu::BufferBindingType],
        images: &[(usize, &'static str)],
    ) -> CachedPipelineRef {
        let mut layout = types.to_vec();
        layout.push(wgpu::BufferBindingType::Uniform);
        let (cached, hit) = self.pipeline_cache.get_or_compile_pointwise(
            &self.device, shader, &layout, images, || {
                let mut source = address_shader(shader, images);
                source.push_str(&format!("\nstruct PointwiseBases {{ values: array<vec4<u32>, 4>, }}\n@group(0) @binding({}) var<uniform> pointwise_bases: PointwiseBases;\n", types.len()));
                source
            });
        self.record_pipeline_cache_lookup(hit);
        cached
    }
}

/// Insert a scalar base only on exact array identifiers. Leave comments and
/// unrelated identifiers untouched, including names that share a suffix.
fn address_shader(source: &str, images: &[(usize, &str)]) -> String {
    let bytes = source.as_bytes();
    let mut result = String::with_capacity(source.len());
    let mut index = 0;
    let mut copied = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if bytes[index..].starts_with(b"/*") {
            index += 2;
            let mut depth = 1;
            while index < bytes.len() && depth != 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            if let Some((binding, _)) = images
                .iter()
                .find(|(_, name)| *name == &source[start..index])
            {
                let mut open = index;
                while open < bytes.len() && bytes[open].is_ascii_whitespace() {
                    open += 1;
                }
                if bytes.get(open) == Some(&b'[') {
                    result.push_str(&source[copied..open + 1]);
                    result.push_str(&format!(
                        "pointwise_bases.values[{}][{}] + ",
                        binding / 4,
                        binding % 4
                    ));
                    copied = open + 1;
                    index = open + 1;
                }
            }
        } else {
            index += 1;
        }
    }
    result.push_str(&source[copied..]);
    result
}

#[cfg(test)]
mod logic_tests {
    use super::*;

    #[test]
    fn shader_specialization_preserves_comments_names_and_nested_indices() {
        let source = "// input[ignored]\n/* input[x] */ raw_input[0] = input [lookup[input[i]]];";
        let result = address_shader(source, &[(1, "input")]);
        assert_eq!(
            result,
            "// input[ignored]\n/* input[x] */ raw_input[0] = input [pointwise_bases.values[0][1] + lookup[input[pointwise_bases.values[0][1] + i]]];"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::paged::ImagePage;
    use super::*;

    struct RestoreLimit(Option<std::ffi::OsString>);
    impl Drop for RestoreLimit {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(value) => std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", value),
                    None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
                }
            }
        }
    }
    const SHADER: &str = "
struct Params { n:u32, lanes:u32, parity:u32, pad:u32 }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> input:array<u32>;
@group(0) @binding(2) var<storage,read_write> output:array<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
 let i=gid.x+gid.y*grid.x*256u; if i>=p.n {return;}
 for(var c=0u;c<p.lanes;c++) { let j=i*p.lanes+c;
   if (input[j]&1u)==p.parity { output[j]=input[j]+100u; }
 }
}";
    fn pages(
        backend: &WgpuBackend,
        layout: &[u32],
        values: &[u32],
        storage: bool,
    ) -> Vec<ImagePage> {
        let mut first = 0;
        layout
            .iter()
            .map(|&n_pixels| {
                let begin = first as usize * 3;
                let end = begin + n_pixels as usize * 3;
                let mut contents = values[begin..end].to_vec();
                contents.extend([0xdead_beef; 3]);
                let mut usage = wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
                if storage {
                    usage |= wgpu::BufferUsages::STORAGE;
                }
                let page = ImagePage {
                    first_pixel: first,
                    n_pixels,
                    buffer: backend
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("view_sentinels"),
                            contents: bytemuck::cast_slice(&contents),
                            usage,
                        }),
                };
                first += n_pixels;
                page
            })
            .collect()
    }
    fn read(backend: &WgpuBackend, pages: &[ImagePage]) -> Vec<u32> {
        let mut result = Vec::new();
        for page in pages {
            let values = backend
                .pipe
                .readback_buffer(&page.buffer, page.buffer.size())
                .unwrap();
            let words: Vec<_> = values.into_iter().map(f32::to_bits).collect();
            let n = page.n_pixels as usize * 3;
            assert_eq!(
                &words[n..],
                &[0xdead_beef; 3],
                "dispatch changed page sentinel"
            );
            result.extend_from_slice(&words[..n]);
        }
        result
    }
    fn execute(backend: &WgpuBackend, plan: &ViewPlan, lanes: u32, parity: u32) {
        let pipeline = backend.cached_pointwise_pipeline(
            SHADER,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            &[(1, "input"), (2, "output")],
        );
        plan.for_each(backend, |encoder, chunk| {
            let params = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("view_test_params"),
                    contents: bytemuck::cast_slice(&[chunk.count, lanes, parity, 0]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let mut bases = [0u32; 16];
            bases[1] = chunk.views[0].base;
            bases[2] = chunk.views[1].base;
            let bases = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("view_test_bases"),
                    contents: bytemuck::cast_slice(&bases),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let group = backend
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("view_test"),
                    layout: &pipeline.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: chunk.views[0].binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: chunk.views[1].binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: bases.as_entire_binding(),
                        },
                    ],
                });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            dispatch_linear(&mut pass, chunk.count);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn independent_scalar_residuals_sparse_writes_fallback_and_replay() {
        let backend = crate::test_support::adapter();
        let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "500");
        }
        let source_values: Vec<u32> = (0..129).collect();
        for lanes in [1u32, 3] {
            for storage in [true, false] {
                let source = pages(&backend, &[43], &source_values, storage);
                let output = pages(&backend, &[5, 38], &[7; 129], storage);
                let plan = ViewPlan::new(
                    &backend,
                    129 / u64::from(lanes),
                    u64::from(lanes) * 4,
                    &[(&source, Access::Read), (&output, Access::WritePartial)],
                    "layout_replay",
                )
                .unwrap();
                assert_eq!(plan.bindings.iter().any(|b| b.scratch.is_some()), !storage);
                if storage && plan.alignment > 4 {
                    let chunk = plan.chunk(if lanes == 1 { 15 } else { 5 }).unwrap();
                    assert_ne!(
                        chunk.views[0].base, chunk.views[1].base,
                        "different page layouts need independent bases"
                    );
                    let unaligned = (0..plan.n)
                        .map(|first| plan.chunk(first).unwrap())
                        .find(|chunk| chunk.views[0].base % 3 != 0)
                        .expect("scalar residual must not be rounded to RGB pixels");
                    assert!(
                        unaligned.views.iter().all(
                            |view| view.offset % plan.alignment == 0 && view.size <= plan.limit
                        )
                    );
                }
                execute(&backend, &plan, lanes, 0);
                let first: Vec<_> = source_values
                    .iter()
                    .map(|&v| if v & 1 == 0 { v + 100 } else { 7 })
                    .collect();
                assert_eq!(read(&backend, &output), first);
                // Reuse the same plan, then complete another sparse owner.
                execute(&backend, &plan, lanes, 0);
                assert_eq!(read(&backend, &output), first);
                execute(&backend, &plan, lanes, 1);
                assert_eq!(
                    read(&backend, &output),
                    source_values.iter().map(|v| v + 100).collect::<Vec<_>>()
                );
                assert_eq!(read(&backend, &source), source_values);
            }
        }
    }
    #[test]
    fn malformed_pages_and_cross_chunk_alias_fail_before_writes() {
        let backend = crate::test_support::adapter();
        let values: Vec<u32> = (0..18).collect();
        let original = pages(&backend, &[3, 3], &values, true);
        let swapped = vec![
            ImagePage {
                first_pixel: 0,
                n_pixels: 3,
                buffer: original[1].buffer.clone(),
            },
            ImagePage {
                first_pixel: 3,
                n_pixels: 3,
                buffer: original[0].buffer.clone(),
            },
        ];
        assert!(
            ViewPlan::new(
                &backend,
                6,
                12,
                &[(&original, Access::Read), (&swapped, Access::Write)],
                "cross_alias"
            )
            .is_err()
        );
        let mut malformed = original.clone();
        malformed[1].first_pixel = 2;
        assert!(
            ViewPlan::new(
                &backend,
                6,
                12,
                &[(&malformed, Access::ReadWrite)],
                "overlap"
            )
            .is_err()
        );
        malformed[1].first_pixel = 4;
        assert!(ViewPlan::new(&backend, 6, 12, &[(&malformed, Access::ReadWrite)], "gap").is_err());
        malformed.swap(0, 1);
        assert!(
            ViewPlan::new(
                &backend,
                6,
                12,
                &[(&malformed, Access::ReadWrite)],
                "unsorted"
            )
            .is_err()
        );
        assert_eq!(read(&backend, &original), values);
        let legal = ViewPlan::new(
            &backend,
            6,
            12,
            &[(&original, Access::ReadWrite)],
            "legal_in_place",
        )
        .unwrap();
        assert!(
            legal
                .bindings
                .iter()
                .all(|binding| binding.scratch.is_none())
        );
        let alias = ViewPlan::new(
            &backend,
            6,
            12,
            &[(&original, Access::Read), (&original, Access::WritePartial)],
            "safe_alias",
        )
        .unwrap();
        assert!(
            alias
                .bindings
                .iter()
                .all(|binding| binding.scratch.is_some())
        );
        execute(&backend, &alias, 3, 0);
        assert_eq!(
            read(&backend, &original),
            values
                .iter()
                .map(|&v| if v & 1 == 0 { v + 100 } else { v })
                .collect::<Vec<_>>()
        );
        let fixed = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fixed_64_bytes"),
                contents: &[0u8; 64],
                usage: wgpu::BufferUsages::STORAGE,
            });
        let fixed_plan = ViewPlan::new(
            &backend,
            6,
            12,
            &[(&fixed, Access::Fixed), (&original, Access::ReadWrite)],
            "fixed_bytes",
        )
        .unwrap();
        assert_eq!(fixed_plan.chunk(0).unwrap().views[0].size, 64);
        const IN_PLACE: &str = "
struct Params { n:u32, pad:vec3<u32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read_write> data:array<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
 let i=gid.x+gid.y*grid.x*256u; if i<p.n { data[i]=data[i]+1u; }
}";
        super::super::highlight::pointwise_job(
            &backend,
            18,
            4,
            IN_PLACE,
            &[18u32, 0, 0, 0, 0, 0, 0, 0],
            &[(&original, Access::ReadWrite)],
            None,
            "legal_read_write",
        )
        .unwrap()
        .execute(&backend)
        .unwrap();
        assert_eq!(
            read(&backend, &original),
            values
                .iter()
                .map(|&v| if v & 1 == 0 { v + 101 } else { v + 1 })
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn cancellation_stops_new_chunks_and_replays_from_immutable_source() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let backend = crate::test_support::adapter();
        let token = Arc::new(AtomicBool::new(false));
        let mut cancelled = backend.derived_explicit();
        cancelled.pipe = backend.pipe.with_cancellation(Some(token.clone()));
        cancelled.cancellation = Some(token.clone());
        let values: Vec<u32> = (0..12).collect();
        let source = pages(&backend, &[2, 2], &values, true);
        let output = pages(&backend, &[2, 2], &[0; 12], true);
        let plan = ViewPlan::new(
            &backend,
            4,
            12,
            &[(&source, Access::Read), (&output, Access::Write)],
            "cancel_replay",
        )
        .unwrap();
        let copy = |encoder: &mut ObservedEncoder, chunk: &ViewChunk| {
            let a = &chunk.views[0];
            let b = &chunk.views[1];
            encoder.copy_buffer_to_buffer(
                &a.buffer,
                a.offset + u64::from(a.base) * 4,
                &b.buffer,
                b.offset + u64::from(b.base) * 4,
                u64::from(chunk.count) * 12,
            );
        };
        let mut encoded = 0;
        let result = plan.for_each(&cancelled, |encoder, chunk| {
            encoded += 1;
            copy(encoder, chunk);
            if encoded == 2 {
                token.store(true, Ordering::Relaxed);
            }
            Ok(())
        });
        assert_eq!(result.unwrap_err(), "operation cancelled");
        assert_eq!(encoded, 2);
        assert_eq!(
            read(&backend, &output),
            [values[..6].to_vec(), vec![0; 6]].concat()
        );
        assert_eq!(read(&backend, &source), values);
        token.store(false, Ordering::Relaxed);
        plan.for_each(&cancelled, |encoder, chunk| {
            copy(encoder, chunk);
            Ok(())
        })
        .unwrap();
        assert_eq!(read(&backend, &output), values);
    }
}
