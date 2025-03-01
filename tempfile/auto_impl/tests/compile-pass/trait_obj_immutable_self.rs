use auto_impl::auto_impl;


#[auto_impl(&, &mut, Box, Rc, Arc)]
trait Trait {
    fn foo(&self);
}

fn assert_impl<T: Trait>() {}

fn main() {
    use std::{rc::Rc, sync::Arc};

    assert_impl::<&dyn Trait>();
    assert_impl::<&mut dyn Trait>();
    assert_impl::<Box<dyn Trait>>();
    assert_impl::<Rc<dyn Trait>>();
    assert_impl::<Arc<dyn Trait>>();
}
