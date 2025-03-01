#[impl_trait_for_tuples::impl_for_tuples(3, 5)]
trait Test {}

struct Impl;

impl Test for Impl {}

fn test<T: Test>() {}
fn main() {
    test::<(Impl, Impl)>();
    test::<(Impl, Impl, Impl)>();
}
