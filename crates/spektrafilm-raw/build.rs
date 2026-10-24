use std::{env, fs, path::PathBuf};

// pkg-config's Library discards compiler-driver switches such as -fopenmp.
// Query the same target package for those switches so the scoped X-Trans
// guard controls the runtime selected by LibRaw, including downstream links.
fn pkg_config_env(name: &str) -> Option<std::ffi::OsString> {
    let target = env::var("TARGET").expect("Cargo TARGET");
    let kind = if env::var("HOST").expect("Cargo HOST") == target {
        "HOST"
    } else {
        "TARGET"
    };
    [
        format!("{name}_{target}"),
        format!("{name}_{}", target.replace('-', "_")),
        format!("{kind}_{name}"),
        name.to_owned(),
    ]
    .iter()
    .find_map(|key| {
        println!("cargo:rerun-if-env-changed={key}");
        env::var_os(key)
    })
}

fn configure_libraw_openmp(build: &mut cc::Build, library: &pkg_config::Library) {
    let mut command = std::process::Command::new(
        pkg_config_env("PKG_CONFIG").unwrap_or_else(|| "pkg-config".into()),
    );
    for name in [
        "PKG_CONFIG_PATH",
        "PKG_CONFIG_LIBDIR",
        "PKG_CONFIG_SYSROOT_DIR",
    ] {
        if let Some(value) = pkg_config_env(name) {
            command.env(name, value);
        }
    }
    if env::var_os("LIBRAW_STATIC").is_some()
        || (env::var_os("LIBRAW_DYNAMIC").is_none()
            && env::var_os("PKG_CONFIG_ALL_STATIC").is_some())
    {
        command.arg("--static");
    }
    let output = command
        .args(["--libs-only-other", "libraw"])
        .output()
        .expect("query selected LibRaw OpenMP flags");
    assert!(
        output.status.success(),
        "query selected LibRaw OpenMP flags: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let flags =
        String::from_utf8(output.stdout).expect("LibRaw package linker flags must be UTF-8");
    let explicit_runtime = library
        .libs
        .iter()
        .find(|name| ["gomp", "omp", "iomp5", "vcomp"].contains(&name.as_str()));
    let openmp = flags.split_whitespace().find(|flag| {
        *flag == "-fopenmp" || flag.starts_with("-fopenmp=") || flag.starts_with("/openmp")
    });
    let runtime = explicit_runtime.map(String::as_str).or_else(|| openmp.map(|flag| match flag {
        "-fopenmp=libgomp" => "gomp",
        "-fopenmp=libomp" => "omp",
        "-fopenmp=libiomp5" => "iomp5",
        "-fopenmp" => {
            assert!(build.get_compiler().is_like_gnu(), "LibRaw advertises ambiguous -fopenmp with a non-GNU bridge compiler; name its actual OpenMP runtime in libraw.pc (-lgomp, -lomp, -liomp5, or -fopenmp=lib...) so ordered X-Trans controls the same runtime");
            "gomp"
        },
        flag if flag.starts_with("/openmp") => "vcomp",
        _ => panic!("unsupported LibRaw OpenMP runtime flag: {flag}"),
    }));
    if let Some(runtime) = runtime {
        build.define("SF_RAW_OPENMP", None);
        if explicit_runtime.is_none() {
            println!("cargo:rustc-link-lib={runtime}");
        }
    }
}

fn link_libraw(library: &pkg_config::Library) {
    let target = env::var("TARGET").expect("Cargo TARGET");
    let windows = target.contains("windows");
    let msvc = target.contains("msvc");
    let apple = target.contains("apple");
    let static_link = env::var_os("LIBRAW_STATIC").is_some()
        || (env::var_os("LIBRAW_DYNAMIC").is_none()
            && env::var_os("PKG_CONFIG_ALL_STATIC").is_some());
    let names: Vec<&str> = if static_link {
        if msvc {
            vec!["raw.lib", "libraw.lib"]
        } else {
            vec!["libraw.a"]
        }
    } else if msvc {
        vec!["raw.lib", "libraw.lib"]
    } else if windows {
        vec!["libraw.dll.a", "raw.dll.a", "libraw.a"]
    } else if apple {
        vec!["libraw.dylib", "libraw.a"]
    } else {
        vec!["libraw.so", "libraw.a"]
    };
    let selected = library
        .link_files
        .iter()
        .find(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                names.contains(&name.as_ref())
                    || (!static_link
                        && (name.starts_with("libraw.so.")
                            || (name.starts_with("libraw.") && name.ends_with(".dylib"))))
            })
        })
        .cloned()
        .or_else(|| {
            library.link_paths.iter().find_map(|directory| {
                names
                    .iter()
                    .map(|name| directory.join(name))
                    .find(|path| path.is_file())
            })
        })
        .unwrap_or_else(|| {
            panic!(
                "LibRaw {} has no linkable library in pkg-config paths {:?}; expected {:?}",
                library.version, library.link_paths, names
            )
        });
    let selected = fs::canonicalize(&selected).expect("resolve selected LibRaw library");
    let is_static = static_link
        || (selected.extension().is_some_and(|ext| ext == "a")
            && !selected.to_string_lossy().ends_with(".dll.a"));
    // Cargo merges native search paths across crates. Give this link artifact a
    // unique name so OpenImageIO's system -L cannot select an older LibRaw ABI.
    // The shared library's SONAME/install name still controls runtime loading.
    let filename = if apple {
        format!(
            "libspektrafilm_selected_raw.{}",
            if is_static { "a" } else { "dylib" }
        )
    } else {
        format!(
            "spektrafilm_selected_{}",
            selected.file_name().unwrap().to_string_lossy()
        )
    };
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let staged = out.join(&filename);
    if staged.exists() || staged.symlink_metadata().is_ok() {
        fs::remove_file(&staged).expect("replace selected LibRaw link artifact");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&selected, &staged).expect("stage selected LibRaw link artifact");
    #[cfg(not(unix))]
    fs::copy(&selected, &staged).expect("stage selected LibRaw import library");
    println!("cargo:rerun-if-changed={}", selected.display());
    println!("cargo:rustc-link-search=native={}", out.display());
    let kind = if is_static { "static" } else { "dylib" };
    if apple {
        println!("cargo:rustc-link-lib={kind}=spektrafilm_selected_raw");
    } else {
        let name = if msvc {
            staged.to_string_lossy().into_owned()
        } else {
            filename
        };
        println!("cargo:rustc-link-lib={kind}:+verbatim={name}");
    }
    for path in &library.link_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    for name in &library.libs {
        if name != "raw" && !(msvc && ["m", "c", "pthread"].contains(&name.as_str())) {
            let name = if apple && name == "stdc++" {
                "c++"
            } else {
                name.as_str()
            };
            println!("cargo:rustc-link-lib={name}");
        }
    }
    for path in &library.link_files {
        if fs::canonicalize(path).ok().as_ref() != Some(&selected) {
            println!(
                "cargo:rustc-link-search=native={}",
                path.parent().unwrap().display()
            );
            println!(
                "cargo:rustc-link-lib=dylib:+verbatim={}",
                path.file_name().unwrap().to_string_lossy()
            );
        }
    }
    for path in &library.framework_paths {
        println!("cargo:rustc-link-search=framework={}", path.display());
    }
    for name in &library.frameworks {
        println!("cargo:rustc-link-lib=framework={name}");
    }
    for args in &library.ld_args {
        println!("cargo:rustc-link-arg=-Wl,{}", args.join(","));
    }
}

fn main() {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("native/raw.cpp")
        .file("native/lens.cpp");
    for package in ["libraw", "lensfun", "exiv2", "glib-2.0"] {
        let mut config = pkg_config::Config::new();
        if package == "libraw" {
            config.atleast_version("0.22.0").cargo_metadata(false);
        }
        let library = config.probe(package).unwrap_or_else(|error| {
            panic!("spektrafilm-raw requires native {package} development files: {error}. See README native dependencies and PKG_CONFIG_PATH.")
        });
        if package == "libraw" {
            link_libraw(&library);
            configure_libraw_openmp(&mut build, &library);
        }
        for include in library.include_paths {
            build.include(include);
        }
    }
    build.compile("spektrafilm_raw_native");
    println!("cargo:rerun-if-changed=native/raw.cpp");
    println!("cargo:rerun-if-changed=native/lens.cpp");
    println!("cargo:rerun-if-changed=native/float_reference.cpp");
}
