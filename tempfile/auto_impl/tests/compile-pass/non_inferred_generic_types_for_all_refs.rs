use auto_impl::auto_impl;

#[auto_impl(&)]
trait Foo {
    fn foo<T>();
    fn bar<U>(&self);
}


fn main() {}
