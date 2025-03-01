// NOTE: generate fully expanded version with `cargo expand`.
//
//       cargo expand --example readme_example
#![allow(dead_code)]

use derive_builder::Builder;

#[derive(Default, Builder, Debug)]
#[builder(setter(into))]
struct Channel {
    token: i32,
    special_info: i32,
}

fn main() {
    // builder pattern, go, go, go!...
    let ch = ChannelBuilder::default()
        .special_info(42u8)
        .token(19_124)
        .build()
        .unwrap();
    println!("{:?}", ch);
}
