use auto_impl::auto_impl;


trait Foo {}

#[auto_impl(&, &mut)]
impl Foo for usize {}

fn main() {}
