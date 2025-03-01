#![deny(warnings)]

struct Sut;

#[bon::bon]
impl Sut {
    #[builder]
    fn sut(self, #[builder(start_fn)] _x1: u32, _x2: u32) {}
}

fn main() {
    let sut = Sut.sut(99);

    // Previously, there was an attempt to generate names for private fields
    // with randomness to ensure users don't try to access them. This however,
    // conflicts with caching in some build systems. See the following issue
    // for details: https://github.com/elastio/bon/issues/218
    let SutSutBuilder {
        __unsafe_private_phantom: _,
        __unsafe_private_start_fn_args: _,
        __unsafe_private_receiver: _,
        __unsafe_private_named: _,
    } = sut;
}
