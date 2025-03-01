use auto_impl::auto_impl;

#[auto_impl(&)]
trait Foo {
    fn foo<T>();
    fn bar<U>(&self);
    fn baz<'a, U>() -> &'a str;
    fn qux<'a, 'b, 'c, U, V, T>(&self) -> (&'a str, &'b str, &'c str);
}


fn main() {}
