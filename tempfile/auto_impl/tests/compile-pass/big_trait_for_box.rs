use auto_impl::auto_impl;


#[auto_impl(Box)]
trait BoxTrait1<'a, T: for<'b> Into<&'b str>> {
    type Type1;
    type Type2;

    const FOO: u32;

    fn execute1<'b>(&'a self, arg1: &'b T) -> Result<Self::Type1, String>;
    fn execute2(&mut self, arg1: i32) -> Self::Type2;
    fn execute3(self) -> Self::Type1;
    fn execute4(arg1: String) -> Result<i32, String>;
    fn execute5() -> String;
}


fn main() {}
