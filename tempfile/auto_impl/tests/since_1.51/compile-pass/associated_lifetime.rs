#[auto_impl::auto_impl(&, Arc, Box)]
pub trait Trait {
    type Type<'a, T>: Iterator<Item = T> + 'a;
}

fn main() {}