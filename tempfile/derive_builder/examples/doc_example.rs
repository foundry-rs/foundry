// NOTE: generate fully expanded version with `cargo expand`.
//
//       cargo expand --example doc_example

use derive_builder::Builder;

#[allow(dead_code)]
#[derive(Builder)]
struct Lorem {
    ipsum: u32,
}

fn main() {}
