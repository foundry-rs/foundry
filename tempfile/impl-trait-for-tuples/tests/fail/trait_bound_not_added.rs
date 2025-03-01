trait Test {
    fn test();
}

#[impl_trait_for_tuples::impl_for_tuples(5)]
#[tuple_types_no_default_trait_bound]
impl Test for Tuple {
    fn test() {
        for_tuples!( #( Tuple::test(); )* )
    }
}

struct Impl;

impl Test for Impl {}

fn test<T: Test>() {}
fn main() {
    test::<(Impl, Impl)>();
    test::<(Impl, Impl, Impl)>();
}
