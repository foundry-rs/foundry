use bon::Builder;

#[derive(Builder)]
struct WrongName {
    #[builder(field)]
    __x1: i32,
}

fn main() {}
