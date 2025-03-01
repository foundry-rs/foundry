use auto_impl::auto_impl;


#[auto_impl(&, &mut)]
fn foo(s: String) -> u32 {
    3
}

fn main() {}
