trait Test {
    fn test();
}

#[impl_trait_for_tuples::impl_for_tuples(2)]
impl Test for Tuple {
    fn test() {
        for_tuples!( where #( Tuple: Test ),* )
    }
}

fn main() {}
