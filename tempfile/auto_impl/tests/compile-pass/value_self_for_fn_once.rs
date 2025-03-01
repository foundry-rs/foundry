use auto_impl::auto_impl;


#[auto_impl(FnOnce)]
trait Foo {
    fn execute(self);
}

fn foo(_: impl Foo) {}

fn bar() {
    // FnOnce
    let s = String::new();
    foo(|| drop(s));

    // FnMut
    let mut x = 0;
    foo(|| x += 1);

    // Fn
    foo(|| {});
}


fn main() {}
