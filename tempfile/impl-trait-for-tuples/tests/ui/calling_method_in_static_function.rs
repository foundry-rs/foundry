trait Test {
    type Test;

    fn test() -> Self::Test;
}

#[impl_trait_for_tuples::impl_for_tuples(2)]
impl Test for Tuple {
    for_tuples!( type Test = ( #( Tuple::Test ),* ); );

    fn test() -> Self::Test {
        for_tuples!( ( #( Tuple.test() ),* ) )
    }
}

fn main() {}
