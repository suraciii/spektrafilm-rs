fn main() {
    let oiio = pkg_config::Config::new().probe("OpenImageIO").expect("Install OpenImageIO development libraries (libopenimageio-dev, brew install openimageio, or vcpkg openimageio)");
    let exiv = pkg_config::Config::new().probe("exiv2").expect("Install Exiv2 development libraries (libexiv2-dev, brew install exiv2, or vcpkg exiv2)");
    // OpenImageIO.pc can omit public header dependencies. Imath supplies half.h;
    // external fmt installs use a forwarding header instead of bundled headers.
    let imath = pkg_config::Config::new().probe("Imath").expect("Install Imath development libraries (libimath-dev, brew install imath, or mingw-w64-ucrt-x86_64-imath) and expose Imath.pc through PKG_CONFIG_PATH");
    let fmt_header = oiio.include_paths.iter()
        .map(|path| path.join("OpenImageIO/detail/fmt/format.h"))
        .find(|path| path.is_file())
        .expect("OpenImageIO development headers are incomplete: OpenImageIO/detail/fmt/format.h was not found in the OpenImageIO.pc include paths");
    println!("cargo:rerun-if-changed={}", fmt_header.display());
    let fmt_header_contents = std::fs::read_to_string(&fmt_header)
        .unwrap_or_else(|error| panic!("Cannot read OpenImageIO fmt header {}: {error}", fmt_header.display()));
    let fmt = if fmt_header_contents.trim() == "#include <fmt/format.h>" {
        Some(pkg_config::Config::new().cargo_metadata(false).probe("fmt").expect("This OpenImageIO installation uses external fmt headers: install libfmt-dev, brew install fmt, or mingw-w64-ucrt-x86_64-fmt and expose fmt.pc through PKG_CONFIG_PATH"))
    } else {
        None
    };
    let mut build = cc::Build::new();
    build.cpp(true).file("src/image_io_bridge.cpp").flag_if_supported("-std=c++17");
    for path in oiio.include_paths.iter().chain(exiv.include_paths.iter()).chain(imath.include_paths.iter()) { build.include(path); }
    if let Some(fmt) = fmt {
        for path in fmt.include_paths { build.include(path); }
    }
    build.compile("spektrafilm_image_io");
    println!("cargo:rerun-if-changed=src/image_io_bridge.cpp");
}
