use auto_impl::auto_impl;


#[auto_impl(Fn)]
trait FnTrait2<'a, T> {
    fn execute<'b, 'c>(
        &'a self,
        arg1: &'b T,
        arg2: &'c T,
        arg3: &'static str,
    ) -> Result<T, String>;
}


fn main() {}
