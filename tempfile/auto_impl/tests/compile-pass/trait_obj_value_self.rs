use auto_impl::auto_impl;


#[auto_impl(Box)]
trait Trait {
    fn foo(self);
}


fn main() {}
