use auto_impl::auto_impl;


#[auto_impl(&)]
trait Foo {
    #[auto_impl(ferris_for_life)]
    fn a(&self);
}


fn main() {}
