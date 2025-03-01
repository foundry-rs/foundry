use auto_impl::auto_impl;

trait Supi {}

#[auto_impl(Fn)]
trait Foo: Supi {
    fn foo(&self, x: u32) -> String;
}


fn main() {}
