use auto_impl::auto_impl;


struct Data {
    id: usize,
}

#[auto_impl(&, Box)]
trait Foo {
    #[auto_impl(keep_default_for(&))]
    fn foo(&self, ref mut data: Data) {
        data.id += 1;
    }
}


fn main() {}
