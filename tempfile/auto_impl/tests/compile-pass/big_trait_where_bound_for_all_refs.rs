use auto_impl::auto_impl;


#[auto_impl(Arc, Box, Rc, &, &mut)]
trait Big<'a, T: for<'b> Into<&'b str>> {
    type Type1;
    type Type2: std::ops::Deref;

    const FOO: u32;

    fn execute1<'b>(&'a self, arg1: &'b T) -> Result<Self::Type1, String>
    where
        T: Clone,
        <Self::Type2 as std::ops::Deref>::Target: Clone;

    fn execute2(&self) -> Self::Type2
    where
        T: std::ops::Deref<Target = Self::Type1>;
}


fn main() {}
