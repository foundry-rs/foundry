use auto_impl::auto_impl;


#[auto_impl(&)]
trait Foo {
    fn foo(&mut self);
}


fn main() {}
