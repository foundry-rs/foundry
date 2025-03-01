use auto_impl::auto_impl;


#[auto_impl(Rc)]
trait Foo {
    fn foo(&mut self);
}


fn main() {}
