use auto_impl::auto_impl;


#[auto_impl(&, &mut)]
enum Foo {
    None,
    Name(String),
    Rgb(u8, u8, u8),
}

fn main() {}
