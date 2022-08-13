use vergen::{Config, ShaKind};

fn main() {
    let mut config = Config::default();
    // Change the SHA output to the short variant
    *config.git_mut().sha_kind_mut() = ShaKind::Short;
    vergen::vergen(config)
        .unwrap_or_else(|e| panic!("vergen crate failed to generate version information! {e}"));
}
