use auto_impl::auto_impl;

trait Supi {}

#[auto_impl(Box, &)]
trait Foo: Supi {}


fn main() {}
