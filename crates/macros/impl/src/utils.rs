#[inline]
pub fn krate() -> syn::Path {
    syn::parse_str(crate_str()).unwrap()
}

#[inline]
pub fn crate_str() -> &'static str {
    match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(var) if var.ends_with("macros") => "crate",
        _ => "::foundry_macros",
    }
}
