use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, LazyLock, Mutex, atomic::AtomicBool},
};
use wasmi::{CompilationMode, Config, Engine, Linker, Store, TypedResumableCall};
use wasmi_wasi::{
    WasiCtx, add_to_linker,
    sync::{Dir, WasiCtxBuilder, ambient_authority},
};

type ModuleCache = Vec<([u8; 32], std::sync::Weak<Module>)>;
const RESOURCE_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/fotufilm/source-manifest.json"
));

pub(crate) fn verified_bytes(root: &Path, name: &str) -> Result<Vec<u8>, String> {
    let manifest: serde_json::Value =
        serde_json::from_str(RESOURCE_MANIFEST).map_err(|e| e.to_string())?;
    if manifest["upstream_revision"].as_str() != Some(super::SOURCE_REVISION) {
        return Err("Fotufilm resource manifest and native source revisions differ".into());
    }
    let expected = manifest["files"][name]
        .as_str()
        .ok_or_else(|| format!("Fotufilm resource {name} is not pinned"))?;
    let bytes =
        std::fs::read(root.join(name)).map_err(|e| format!("Fotufilm resource {name}: {e}"))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if digest != expected {
        return Err(format!(
            "Fotufilm resource {name} does not match its pinned source digest"
        ));
    }
    Ok(bytes)
}

pub(crate) struct Module {
    engine: Engine,
    module: wasmi::Module,
}
static MODULES: LazyLock<Mutex<ModuleCache>> = LazyLock::new(|| Mutex::new(Vec::new()));
const FUEL_QUANTUM: u64 = 1_000_000;

pub(crate) fn load(root: &Path) -> Result<Arc<Module>, String> {
    let bytes = verified_bytes(root, "preparation.wasm")?;
    let identity: [u8; 32] = Sha256::digest(&bytes).into();
    let mut cache = MODULES
        .lock()
        .map_err(|_| "Fotufilm module cache lock poisoned")?;
    cache.retain(|(_, entry)| entry.strong_count() > 0);
    if let Some(module) = cache
        .iter()
        .find(|(p, _)| p == &identity)
        .and_then(|(_, v)| v.upgrade())
    {
        return Ok(module);
    }
    let mut config = Config::default();
    config
        .consume_fuel(true)
        .compilation_mode(CompilationMode::Eager);
    let engine = Engine::new(&config);
    let module = wasmi::Module::new(&engine, &bytes[..])
        .map_err(|e| format!("Fotufilm preparation module: {e}"))?;
    let module = Arc::new(Module { engine, module });
    cache.push((identity, Arc::downgrade(&module)));
    Ok(module)
}

pub(crate) fn prepare(
    module: &Module,
    root: &Path,
    request: &serde_json::Value,
    scene: Option<&[f32]>,
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    super::check_cancel(cancellation)?;
    verified_bytes(root, "prior/rec2020-reflectance-prior.coeff")?;
    let mut builder = WasiCtxBuilder::new();
    builder
        .env("FOTUFILM_RESOURCES", "/resources")
        .map_err(|e| e.to_string())?;
    let prior = root.join("prior");
    let resource = Dir::open_ambient_dir(&prior, ambient_authority())
        .map_err(|e| format!("Fotufilm resource directory {}: {e}", prior.display()))?;
    builder
        .preopened_dir(resource, "/resources")
        .map_err(|e| e.to_string())?;
    let mut store = Store::new(&module.engine, builder.build());
    store.set_fuel(u64::MAX).map_err(|e| e.to_string())?;
    let mut linker = Linker::<WasiCtx>::new(&module.engine);
    add_to_linker(&mut linker, |ctx| ctx).map_err(|e| e.to_string())?;
    // The original Swift model uses the target's Float libm. WASI's musl can
    // change a receiver solver's accepted steps even from a one-ULP difference.
    for (name, function) in [
        (
            "expf",
            super::native::sf_fotufilm_expf as unsafe extern "C" fn(f32) -> f32,
        ),
        ("exp2f", super::native::sf_fotufilm_exp2f),
        ("logf", super::native::sf_fotufilm_logf),
        ("log1pf", super::native::sf_fotufilm_log1pf),
        ("log2f", super::native::sf_fotufilm_log2f),
        ("log10f", super::native::sf_fotufilm_log10f),
        ("tanf", super::native::sf_fotufilm_tanf),
    ] {
        linker
            .func_wrap("fotufilm_math", name, move |x: f32| {
                // These pure C functions accept all Float values and retain no memory.
                unsafe { function(x) }
            })
            .map_err(|e| e.to_string())?;
    }
    linker
        .func_wrap("fotufilm_math", "powf", |x: f32, y: f32| unsafe {
            super::native::sf_fotufilm_powf(x, y)
        })
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            "fotufilm_native",
            "film_tile_build",
            |mut caller: wasmi::Caller<'_, WasiCtx>,
             texels: i32,
             supersample: i32,
             shape: i32,
             terms: i32,
             count: i32,
             sublayers: i32,
             fractions: i32,
             levels: i32,
             seed: i32,
             record: i32,
             light: i32| {
                build_film_tile(
                    &mut caller,
                    [
                        texels,
                        supersample,
                        shape,
                        terms,
                        count,
                        sublayers,
                        fractions,
                        levels,
                        seed,
                        record,
                        light,
                    ],
                )
            },
        )
        .map_err(|e| e.to_string())?;
    let instance = linker
        .instantiate_and_start(&mut store, &module.module)
        .map_err(|e| format!("Fotufilm preparation instance: {e}"))?;
    if let Ok(init) = instance.get_typed_func::<(), ()>(&store, "_initialize") {
        init.call(&mut store, ()).map_err(|e| e.to_string())?;
    }
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or("Fotufilm preparation module has no memory export")?;
    let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    let count = i32::try_from(bytes.len()).map_err(|_| "Fotufilm request is too large")?;
    let pointer = instance
        .get_typed_func::<i32, i32>(&store, "profile_input")
        .map_err(|e| e.to_string())?
        .call(&mut store, count)
        .map_err(|e| e.to_string())?;
    if pointer == 0 {
        return Err("Fotufilm preparation request allocation failed".into());
    }
    memory
        .write(&mut store, pointer as u32 as usize, &bytes)
        .map_err(|e| e.to_string())?;
    if let Some(scene) = scene {
        let count =
            i32::try_from(scene.len()).map_err(|_| "Fotufilm measurement image is too large")?;
        let pointer = instance
            .get_typed_func::<i32, i32>(&store, "align_scene_input")
            .map_err(|e| e.to_string())?
            .call(&mut store, count)
            .map_err(|e| e.to_string())?;
        if pointer == 0 {
            return Err("Fotufilm scene measurement allocation failed".into());
        }
        let address = pointer as u32 as usize;
        let length = scene
            .len()
            .checked_mul(4)
            .ok_or("Fotufilm scene bytes overflow")?;
        let target = memory
            .data_mut(&mut store)
            .get_mut(
                address
                    ..address
                        .checked_add(length)
                        .ok_or("Fotufilm scene address overflow")?,
            )
            .ok_or("Fotufilm scene memory range is invalid")?;
        for (value, bytes) in scene.iter().zip(target.as_chunks_mut::<4>().0.iter_mut()) {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
    }
    super::check_cancel(cancellation)?;
    store.set_fuel(FUEL_QUANTUM).map_err(|e| e.to_string())?;
    let prepare = instance
        .get_typed_func::<(), i32>(&store, "align_prepare")
        .map_err(|e| e.to_string())?;
    let mut execution = prepare
        .call_resumable(&mut store, ())
        .map_err(|e| format!("Fotufilm preparation failed: {e}"))?;
    let status = loop {
        match execution {
            TypedResumableCall::Finished(status) => break status,
            TypedResumableCall::OutOfFuel(suspended) => {
                super::check_cancel(cancellation)?;
                store
                    .set_fuel(FUEL_QUANTUM.max(suspended.required_fuel()))
                    .map_err(|e| e.to_string())?;
                execution = suspended
                    .resume(&mut store)
                    .map_err(|e| format!("Fotufilm preparation failed: {e}"))?;
            }
            TypedResumableCall::HostTrap(trap) => {
                return Err(format!("Fotufilm preparation host error: {trap:?}"));
            }
        }
    };
    store.set_fuel(u64::MAX).map_err(|e| e.to_string())?;
    let pointer = instance
        .get_typed_func::<(), i32>(&store, "profile_output")
        .map_err(|e| e.to_string())?
        .call(&mut store, ())
        .map_err(|e| e.to_string())?;
    let count = instance
        .get_typed_func::<(), i32>(&store, "profile_output_size")
        .map_err(|e| e.to_string())?
        .call(&mut store, ())
        .map_err(|e| e.to_string())?;
    let count =
        usize::try_from(count).map_err(|_| "Fotufilm preparation returned an invalid length")?;
    let mut output = vec![0; count];
    memory
        .read(&store, pointer as u32 as usize, &mut output)
        .map_err(|e| e.to_string())?;
    if status != 0 {
        return Err(format!("Fotufilm: {}", String::from_utf8_lossy(&output)));
    }
    super::check_cancel(cancellation)?;
    Ok(output)
}

fn build_film_tile(caller: &mut wasmi::Caller<'_, WasiCtx>, args: [i32; 11]) -> i32 {
    let [
        texels,
        supersample,
        shape,
        terms,
        count,
        sublayers,
        fractions,
        levels,
        seed,
        record,
        light,
    ] = args;
    if !(16..=1024).contains(&texels)
        || !(1..=8).contains(&supersample)
        || !(1..=16).contains(&shape)
        || !(1..=8).contains(&count)
        || !(1..=64).contains(&levels)
        || !(0..=2).contains(&record)
    {
        return -1;
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(wasmi::Extern::into_memory)
    else {
        return -1;
    };
    let bytes = memory.data_mut(caller);
    let ranges = [
        (terms, count as usize * 2),
        (sublayers, 4 * 51),
        (fractions, 4 * levels as usize),
        (light, texels as usize * texels as usize * levels as usize),
    ]
    .map(|(offset, floats)| {
        let start = offset as u32 as usize;
        start
            .checked_add(floats * 4)
            .filter(|&end| end <= bytes.len())
            .filter(|_| start.is_multiple_of(4))
            .map(|end| start..end)
    });
    let [Some(terms), Some(sublayers), Some(fractions), Some(light)] = ranges else {
        return -1;
    };
    if !cfg!(target_endian = "little")
        || bytes.as_ptr().align_offset(align_of::<f32>()) != 0
        || [&terms, &sublayers, &fractions]
            .iter()
            .any(|range| range.start < light.end && light.start < range.end)
    {
        return -1;
    }
    // Swift allocates aligned Float arrays. The checked WASM ranges are resident for
    // this synchronous call; no guest allocation or memory growth can occur here.
    // The source builder borrows inputs, owns its graph lock, and retains no buffers.
    unsafe {
        super::native::sf_fotufilm_film_tile_build(
            texels,
            supersample,
            shape,
            bytes.as_ptr().add(terms.start).cast(),
            count,
            bytes.as_ptr().add(sublayers.start).cast(),
            bytes.as_ptr().add(fractions.start).cast(),
            levels,
            seed as u32,
            record,
            bytes.as_mut_ptr().add(light.start).cast(),
        )
    }
}
