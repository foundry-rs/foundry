use std::{env, path::PathBuf};

fn main() {
    let root_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    // Obtain the header files of blst
    let blst_base_dir = root_dir.join("blst");
    let blst_headers_dir = blst_base_dir.join("bindings");

    let c_src_dir = root_dir.join("src");

    let mut cc = cc::Build::new();

    #[cfg(all(windows, target_env = "msvc"))]
    {
        cc.flag("-D_CRT_SECURE_NO_WARNINGS");

        // In blst, if __STDC_VERSION__ isn't defined as c99 or greater, it will typedef a bool to
        // an int. There is a bug in bindgen associated with this. It assumes that a bool in C is
        // the same size as a bool in Rust. This is the root cause of the issues on Windows. If/when
        // this is fixed in bindgen, it should be safe to remove this compiler flag.
        cc.flag("/std:c11");
    }

    cc.include(blst_headers_dir.clone());
    cc.warnings(false);
    cc.file(c_src_dir.join("c_kzg_4844.c"));
    #[cfg(not(debug_assertions))]
    cc.define("NDEBUG", None);

    cc.try_compile("ckzg").expect("Failed to compile ckzg");

    #[cfg(feature = "generate-bindings")]
    {
        let header_path = c_src_dir.join("c_kzg_4844.h");
        let bindings_out_path = root_dir.join("bindings/rust/src/bindings/generated.rs");
        make_bindings(
            header_path.to_str().expect("valid header path"),
            blst_headers_dir.to_str().expect("valid blst header path"),
            &bindings_out_path,
        );
    }

    // Finally, tell cargo this provides ckzg/ckzg_min
    println!("cargo:rustc-link-lib=ckzg");
}

#[cfg(feature = "generate-bindings")]
fn make_bindings(header_path: &str, blst_headers_dir: &str, bindings_out_path: &std::path::Path) {
    use bindgen::Builder;

    #[derive(Debug)]
    struct Callbacks;
    impl bindgen::callbacks::ParseCallbacks for Callbacks {
        fn int_macro(&self, name: &str, _value: i64) -> Option<bindgen::callbacks::IntKind> {
            match name {
                "FIELD_ELEMENTS_PER_BLOB"
                | "BYTES_PER_COMMITMENT"
                | "BYTES_PER_PROOF"
                | "BYTES_PER_FIELD_ELEMENT"
                | "BYTES_PER_BLOB" => Some(bindgen::callbacks::IntKind::Custom {
                    name: "usize",
                    is_signed: false,
                }),
                _ => None,
            }
        }
    }

    let bindings = Builder::default()
        /*
         * Header definitions.
         */
        .header(header_path)
        .clang_args([format!("-I{blst_headers_dir}")])
        // Get bindings only for the header file.
        .allowlist_file(".*c_kzg_4844.h")
        /*
         * Cleanup instructions.
         */
        // Remove stdio definitions related to FILE.
        .opaque_type("FILE")
        // Remove the definition of FILE to use the libc one, which is more convenient.
        .blocklist_type("FILE")
        // Inject rust code using libc's FILE
        .raw_line("use libc::FILE;")
        // Do no generate layout tests.
        .layout_tests(false)
        // Extern functions do not need individual extern blocks.
        .merge_extern_blocks(true)
        // We implement Drop for this type. Copy is not allowed for types with destructors.
        .no_copy("KZGSettings")
        /*
         * API improvements.
         */
        // Do not create individual constants for enum variants.
        .rustified_enum("C_KZG_RET")
        // Make constants used as sizes `usize`.
        .parse_callbacks(Box::new(Callbacks))
        // Add PartialEq and Eq impls to types.
        .derive_eq(true)
        // All types are hashable.
        .derive_hash(true)
        // Blobs are big, we don't want rust to liberally copy them around.
        .no_copy("Blob")
        // Do not make fields public. If we want to modify them we can create setters/mutable
        // getters when necessary.
        .default_visibility(bindgen::FieldVisibilityKind::Private)
        // Blocklist this type alias to use a custom implementation. If this stops being a type
        // alias this line needs to be removed.
        .blocklist_type("KZGCommitment")
        // Blocklist this type alias to use a custom implementation. If this stops being a type
        // alias this line needs to be removed.
        .blocklist_type("KZGProof")
        /*
         * Re-build instructions
         */
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .unwrap();

    let mut bindings = bindings.to_string();
    bindings = replace_ckzg_ret_repr(bindings);
    std::fs::write(bindings_out_path, bindings).expect("Failed to write bindings");
}

// Here we hardcode the C_KZG_RET enum to use C representation. Bindgen
// will use repr(u32) on Unix and repr(i32) on Windows. We would like to
// use the same generated bindings for all platforms. This can be removed
// if/when bindgen fixes this or allows us to choose our own representation
// for types. Using repr(C) is equivalent to repr(u*) for fieldless enums,
// so this should be safe to do. The alternative was to modify C_KZG_RET in
// C, and we decided this was the lesser of two evils. There should be only
// one instance where repr(C) isn't used: C_KZG_RET.
// See: https://github.com/rust-lang/rust-bindgen/issues/1907
#[cfg(feature = "generate-bindings")]
fn replace_ckzg_ret_repr(mut bindings: String) -> String {
    let target = env::var("TARGET").unwrap_or_default();
    let repr_to_replace = if target.contains("windows") {
        "#[repr(i32)]"
    } else {
        "#[repr(u32)]"
    };

    // Find `repr_to_replace` as an attribute of `enum C_KZG_RET`.
    let ckzg_ret = bindings
        .find("enum C_KZG_RET")
        .expect("Could not find C_KZG_RET in bindings");
    let repr_start = bindings[..ckzg_ret]
        .rfind(repr_to_replace)
        .expect("Could not find repr to replace in bindings");

    // Sanity check that it's an attribute of `C_KZG_RET` and not another type.
    assert!(repr_start > bindings[..ckzg_ret].rfind('}').unwrap());

    bindings.replace_range(repr_start..repr_start + repr_to_replace.len(), "#[repr(C)]");

    bindings
}
