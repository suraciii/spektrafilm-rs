fn main() {
    let oiio = pkg_config::Config::new().probe("OpenImageIO").expect("Install OpenImageIO development libraries (libopenimageio-dev, brew install openimageio, or vcpkg openimageio)");
    let exiv = pkg_config::Config::new().probe("exiv2").expect("Install Exiv2 development libraries (libexiv2-dev, brew install exiv2, or vcpkg exiv2)");
    let mut build = cc::Build::new();
    build.cpp(true).file("src/image_io_bridge.cpp").flag_if_supported("-std=c++17");
    for path in oiio.include_paths.iter().chain(exiv.include_paths.iter()) { build.include(path); }
    build.compile("spektrafilm_image_io");
    println!("cargo:rerun-if-changed=src/image_io_bridge.cpp");
}
