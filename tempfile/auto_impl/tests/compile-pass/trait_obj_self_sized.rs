use auto_impl::auto_impl;


#[auto_impl(Box)]
trait Foo: Sized {
    fn foo(&self);
}

#[auto_impl(Box)]
trait Bar where Self: Sized {
    fn foo(&self);
}

#[auto_impl(Box)]
trait Baz: Sized where Self: Sized {
    fn foo(&self);
}


fn main() {}
