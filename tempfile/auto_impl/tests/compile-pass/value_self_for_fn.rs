use auto_impl::auto_impl;


#[auto_impl(Fn)]
trait Foo {
    fn execute(self);
}

fn foo(_: impl Foo) {}

fn bar() {
    // Fn
    foo(|| {});
}


fn main() {}
