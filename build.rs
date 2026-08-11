use std::{env, path::PathBuf};

fn main() {
    let target = env::var("TARGET").unwrap_or_else(|error| panic!("{}", error));

    if target.contains("darwin") || target.contains("ios") {
        println!("cargo:rustc-link-lib=framework=MetalKit");
    }
    if target.contains("aarch64-linux-android") {
        const SWAPPY_DIR: &str = "MINIQUAD_SWAPPY_LIB_DIR";
        println!("cargo:rerun-if-env-changed={SWAPPY_DIR}");

        let dir = PathBuf::from(env::var_os(SWAPPY_DIR).unwrap_or_else(|| {
            panic!(
                "{} must point to the directory containing libswappy.so",
                SWAPPY_DIR
            )
        }));
        let library = dir.join("libswappy.so");
        if !library.is_file() {
            panic!("{} does not exist", library.display());
        }

        println!("cargo:rerun-if-changed={}", library.display());
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=dylib=swappy");
    }
}
