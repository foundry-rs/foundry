use auto_impl::auto_impl;


#[auto_impl(Fn)]
trait Greeter {
    fn greet<const N: usize>(&self, id: usize);
}


fn main() {}
