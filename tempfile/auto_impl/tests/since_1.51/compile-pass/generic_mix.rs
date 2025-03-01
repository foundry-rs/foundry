use auto_impl::auto_impl;

#[auto_impl(&, &mut)]
trait MyTrait<'a, T> {
    fn execute<'b, U, const N: usize>(&'a self, arg1: &'b T, arg2: &'static str, arg3: U) -> Result<(), String>;
}


fn main() {}
