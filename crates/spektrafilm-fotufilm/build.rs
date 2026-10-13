use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo:rerun-if-env-changed=HALIDE_ROOT");
    println!("cargo:rerun-if-env-changed=HALIDE_LIB_DIR");
    println!("cargo:rerun-if-env-changed=HALIDE_LIB_NAME");
    println!("cargo:rerun-if-env-changed=HALIDE_CXX");
    println!("cargo:rerun-if-env-changed=HALIDE_LINKER");
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-changed=vendor/fotufilm");
    let source = Path::new("vendor/fotufilm/Sources/FotufilmHalide");
    let mut build = cc::Build::new();
    let windows_gnu = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    if windows_gnu {
        // Only this C++ graph uses the official SDK's MSVC ABI. Rust and the existing
        // RAW/OIIO dependencies retain their GNU/UCRT toolchain across the C boundary.
        build
            .target("x86_64-pc-windows-msvc")
            .compiler(env::var("HALIDE_CXX").unwrap_or_else(|_| "clang-cl".into()))
            .cargo_metadata(false)
            .flag("/EHsc")
            .flag("/MD");
    }
    build
        .cpp(true)
        .std("c++17")
        .define("FOTUFILM_HALIDE_ENABLED", None)
        .include(source)
        .include(source.join("include"))
        .include("native")
        .file("native/bridge.cpp")
        .file(source.join("FotufilmHalide.cpp"))
        .file(source.join("FotufilmTransport.cpp"))
        .file(source.join("FotufilmFilmTileBuild.cpp"));
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build.flag("/EHsc");
    }
    let include_paths;
    if let Some(root) = env::var_os("HALIDE_ROOT") {
        let root = PathBuf::from(root);
        let include = root.join("include");
        if !include.join("Halide.h").is_file() {
            panic!(
                "HALIDE_ROOT must point to the Halide 21 distribution containing include/Halide.h"
            );
        }
        build.include(&include);
        include_paths = vec![include];
        let library = env::var_os("HALIDE_LIB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if root.join("lib").is_dir() {
                    root.join("lib")
                } else {
                    root.join("lib64")
                }
            });
        if !library.is_dir() {
            panic!("Halide library directory is missing: {}", library.display());
        }
        if !windows_gnu {
            println!("cargo:rustc-link-search=native={}", library.display());
            println!(
                "cargo:rustc-link-lib=dylib={}",
                env::var("HALIDE_LIB_NAME").unwrap_or_else(|_| "Halide".into())
            );
        }
        if env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
            println!("cargo:rustc-link-lib=pthread");
            if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
                println!("cargo:rustc-link-lib=dl");
            }
        }
    } else {
        let library = pkg_config::Config::new().range_version("21".."22").probe("Halide")
            .or_else(|_| pkg_config::Config::new().range_version("21".."22").probe("halide"))
            .unwrap_or_else(|error| panic!("Fotufilm requires official Halide 21 headers and library. Set HALIDE_ROOT or install its pkg-config metadata: {error}"));
        for include in &library.include_paths {
            build.include(include);
        }
        include_paths = library.include_paths;
    }
    // HALIDE_ROOT has no pkg-config version gate: its official header is authoritative.
    let header = include_paths
        .iter()
        .map(|p| p.join("Halide.h"))
        .find(|p| p.is_file())
        .expect("Halide include paths must contain Halide.h");
    let text = fs::read_to_string(&header).expect("reading official Halide header");
    let major = text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        if words.next()? == "#define" && words.next()? == "HALIDE_VERSION_MAJOR" {
            words.next()?.parse::<u32>().ok()
        } else {
            None
        }
    });
    if major != Some(21) {
        panic!(
            "Fotufilm requires Halide 21, found {major:?} in {}",
            header.display()
        );
    }
    build.compile("spektrafilm_fotufilm_native");
    if windows_gnu {
        let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
        let exports = "EXPORTS\nsf_fotufilm_process\nsf_fotufilm_configuration_count\nsf_fotufilm_lut_float_count\nsf_fotufilm_tile_float_count\nsf_fotufilm_source_revision\nsf_fotufilm_expf\nsf_fotufilm_exp2f\nsf_fotufilm_logf\nsf_fotufilm_log1pf\nsf_fotufilm_log2f\nsf_fotufilm_log10f\nsf_fotufilm_powf\nsf_fotufilm_tanf\nsf_fotufilm_film_tile_build\n";
        let def = out.join("spektrafilm-fotufilm.def");
        fs::write(&def, exports).expect("writing Fotufilm C exports");
        let root = PathBuf::from(
            env::var_os("HALIDE_ROOT").expect("official Windows Halide requires HALIDE_ROOT"),
        );
        let libdir = env::var_os("HALIDE_LIB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("lib"));
        let dll = out.join("spektrafilm-fotufilm.dll");
        let status = std::process::Command::new(
            env::var("HALIDE_LINKER").unwrap_or_else(|_| "link.exe".into()),
        )
        .arg("/DLL")
        .arg(format!("/OUT:{}", dll.display()))
        .arg(format!("/DEF:{}", def.display()))
        .arg(format!(
            "/WHOLEARCHIVE:{}",
            out.join("spektrafilm_fotufilm_native.lib").display()
        ))
        .arg(libdir.join("Halide.lib"))
        .status()
        .expect("invoking MSVC linker for native C ABI DLL");
        assert!(
            status.success(),
            "linking official Halide native C ABI DLL failed"
        );
        let status = std::process::Command::new("dlltool")
            .arg("-d")
            .arg(&def)
            .arg("-D")
            .arg("spektrafilm-fotufilm.dll")
            .arg("-l")
            .arg(out.join("libspektrafilm_fotufilm_native.dll.a"))
            .status()
            .expect("creating GNU C ABI import library");
        assert!(
            status.success(),
            "creating native GNU import library failed"
        );
        let profile = env::var("PROFILE").expect("Cargo PROFILE");
        let profile_dir = out
            .ancestors()
            .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(profile.as_str()))
            .expect("Cargo output path contains profile directory");
        fs::copy(&dll, profile_dir.join("spektrafilm-fotufilm.dll"))
            .expect("publishing native C ABI DLL beside binaries");
        fs::create_dir_all(profile_dir.join("deps"))
            .expect("creating Cargo native runtime directory");
        fs::copy(&dll, profile_dir.join("deps/spektrafilm-fotufilm.dll"))
            .expect("publishing native runtime beside tests");
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=dylib=spektrafilm_fotufilm_native");
    }
}
