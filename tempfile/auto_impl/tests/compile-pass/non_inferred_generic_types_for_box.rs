use auto_impl::auto_impl;

#[auto_impl(Box)]
trait Foo {
    fn foo<T>();
    fn bar<U>(&self);
    fn baz<V>(&mut self);
    fn qux<W>(self);
}


fn main() {}
