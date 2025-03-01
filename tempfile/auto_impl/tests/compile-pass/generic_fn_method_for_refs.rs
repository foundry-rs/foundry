use auto_impl::auto_impl;

#[auto_impl(&, Arc)]
pub trait OrderStoreFilter {
    fn filter<F>(&self, predicate: F) -> Result<usize, ()>
    where
        F: Fn(&str) -> bool;
}


fn main() {}
