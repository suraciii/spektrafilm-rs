use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn collect(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("benchmark source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            collect(&path, out)
        } else {
            out.push(path)
        }
    }
}
fn main() {
    let mut files = vec![
        PathBuf::from("Cargo.toml"),
        PathBuf::from("build.rs"),
        PathBuf::from("../../Cargo.lock"),
    ];
    for directory in ["src", "native", "assets"] {
        collect(Path::new(directory), &mut files)
    }
    files.sort();
    let mut hash = Sha256::new();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path.to_string_lossy();
        let bytes = fs::read(&path).expect("immutable benchmark source");
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes)
    }
    println!(
        "cargo:rustc-env=SPEKTRAFILM_BENCHMARK_REVISION={:x}",
        hash.finalize()
    );
    cc::Build::new()
        .cpp(true)
        .file("native/flip_bridge.cpp")
        .flag("-isystemnative/vendor")
        .std("c++17")
        .flag_if_supported("-fno-fast-math")
        .compile("spektrafilm_flip");
}
