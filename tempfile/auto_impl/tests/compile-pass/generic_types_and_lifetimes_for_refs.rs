use auto_impl::auto_impl;

#[auto_impl(&, &mut)]
trait MyTrait<'a, T> {
    fn execute<'b>(&'a self, arg1: &'b T, arg2: &'static str) -> Result<(), String>;
}


fn main() {}
