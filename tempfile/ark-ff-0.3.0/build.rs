extern crate rustc_version;
use rustc_version::{version, version_meta, Channel, Version};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let is_nightly = version_meta().expect("nightly check failed").channel == Channel::Nightly;

    let should_use_asm = cfg!(all(
        feature = "asm",
        target_feature = "bmi2",
        target_feature = "adx",
        target_arch = "x86_64"
    )) && is_nightly;
    if should_use_asm {
        println!("cargo:rustc-cfg=use_asm");
    }

    // TODO: remove this once RFC 2495 ships
    if version().expect("Installed rustc version unparseable!") < Version::parse("1.51.0").unwrap()
    {
        panic!("This code base uses const generics and requires a Rust compiler version greater or equal to 1.51.0");
    }
}
