use auto_impl::auto_impl;


#[auto_impl(&)]
trait Foo {
    fn foo<const I: i32>(&self);
}


fn main() {}
