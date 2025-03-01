trait Test {
    type Test;
}

#[impl_trait_for_tuples::impl_for_tuples(1)]
impl Test for Tuple {
    for_tuples!( type Test = #( Tuple::Test ),*; );
}

fn main() {}
