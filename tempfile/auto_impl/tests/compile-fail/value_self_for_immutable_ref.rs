use auto_impl::auto_impl;


#[auto_impl(&)]
trait Foo {
    fn foo(self);
}


fn main() {}
