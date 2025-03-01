use derive_builder::Builder;

#[derive(Builder)]
#[builder(build_fn(error(validation_error = false, path = "hello")))]
struct Fee {
    hi: u32,
}

#[derive(Builder)]
#[builder(build_fn(error(validation_error = false), validate = "hello"))]
struct Foo {
    hi: u32,
}

#[derive(Builder)]
#[builder(build_fn(error(path = "hello")))]
struct Fii {
    hi: u32,
}

#[derive(Builder)]
#[builder(build_fn(error()))]
struct Fum {
    hi: u32,
}

fn main() {}