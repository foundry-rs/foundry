use auto_impl::auto_impl;


#[auto_impl(&, &mut)]
struct Foo {
    x: usize,
    bar: String,
}


fn main() {}
