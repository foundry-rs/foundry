trait Test {
    type Arg;

    fn test(arg: Self::Arg);
}

trait Custom {
    type NewArg;

    fn new_test(arg: Self::NewArg);
}

#[impl_trait_for_tuples::impl_for_tuples(5)]
#[tuple_types_custom_trait_bound(Custom, Clone)]
impl Test for Tuple {
    for_tuples!( type Arg = ( #( Tuple::NewArg ),* ); );
    fn test(arg: Self::Arg) {
        for_tuples!( #( Tuple::new_test(arg.Tuple); )* );
    }
}

struct Impl;

impl Custom for Impl {
    type NewArg = ();

    fn new_test(_arg: Self::NewArg) {}
}

fn test<T: Test>() {}
fn main() {
    test::<(Impl, Impl)>();
    test::<(Impl, Impl, Impl)>();
}
