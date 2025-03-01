#[auto_impl::auto_impl(&, &mut, Arc, Box, Rc)]
trait AsyncTrait {
    async fn foo(&self);
}

fn main() {}
