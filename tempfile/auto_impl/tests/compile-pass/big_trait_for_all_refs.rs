use auto_impl::auto_impl;


#[auto_impl(Arc, Box, Rc, &, &mut)]
trait RefTrait1<'a, T: for<'b> Into<&'b str>> {
    type Type1;
    type Type2;

    const FOO: u32;

    fn execute1<'b>(&'a self, arg1: &'b T) -> Result<Self::Type1, String>;
    fn execute2(&self) -> Self::Type2;
}


fn main() {}
