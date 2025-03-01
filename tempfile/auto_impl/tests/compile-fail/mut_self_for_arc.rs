use auto_impl::auto_impl;


#[auto_impl(Arc)]
trait Foo {
    fn foo(&mut self);
}


fn main() {}
