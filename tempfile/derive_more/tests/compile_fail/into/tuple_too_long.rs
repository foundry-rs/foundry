#[derive(derive_more::Into)]
#[into((i16, i16, i16))]
struct Point {
    x: i32,
    y: i32,
}

fn main() {}
