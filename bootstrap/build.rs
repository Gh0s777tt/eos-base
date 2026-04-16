use std::env;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    println!("cargo::rustc-link-arg=-z");
    println!("cargo::rustc-link-arg=max-page-size=4096");
    println!("cargo::rustc-link-arg=-T");
    println!("cargo::rustc-link-arg={manifest_dir}/src/{arch}.ld");
}
