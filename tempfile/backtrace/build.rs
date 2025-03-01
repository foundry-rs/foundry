extern crate cc;

use std::env;
use std::path::Path;

// Must be public so the build script of `std` can call it.
pub fn main() {
    match env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        "android" => build_android(),
        _ => {}
    }
}

// Used to detect the value of the `__ANDROID_API__`
// builtin #define
const MARKER: &str = "BACKTRACE_RS_ANDROID_APIVERSION";
const ANDROID_API_C: &str = "
BACKTRACE_RS_ANDROID_APIVERSION __ANDROID_API__
";

fn build_android() {
    // Create `android-api.c` on demand.
    // Required to support calling this from the `std` build script.
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let android_api_c = Path::new(&out_dir).join("android-api.c");
    std::fs::write(&android_api_c, ANDROID_API_C).unwrap();

    let expansion = match cc::Build::new().file(&android_api_c).try_expand() {
        Ok(result) => result,
        Err(e) => {
            eprintln!("warning: android version detection failed while running C compiler: {e}");
            return;
        }
    };
    let expansion = match std::str::from_utf8(&expansion) {
        Ok(s) => s,
        Err(_) => return,
    };
    eprintln!("expanded android version detection:\n{expansion}");
    let i = match expansion.find(MARKER) {
        Some(i) => i,
        None => return,
    };
    let version = match expansion[i + MARKER.len() + 1..].split_whitespace().next() {
        Some(s) => s,
        None => return,
    };
    let version = match version.parse::<u32>() {
        Ok(n) => n,
        Err(_) => return,
    };
    if version >= 21 {
        println!("cargo:rustc-cfg=feature=\"dl_iterate_phdr\"");
    }
}
