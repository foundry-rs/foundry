use auto_impl::auto_impl;


#[auto_impl(FnMut)]
trait Foo {
    fn execute(&mut self);
}

fn foo(_: impl Foo) {}

fn bar() {
    // FnMut
    let mut x = 0;
    foo(|| x += 1);

    // Fn
    foo(|| {});
}


fn main() {}
