
fn foo() {
    use auto_impl::auto_impl;

    #[auto_impl(Fn)]
    trait Foo<'a, T> {
        fn execute<'b>(
            &'a self,
            arg1: &'b T,
            arg3: &'static str,
        ) -> Result<T, String>;
    }

    #[auto_impl(&, &mut, Box, Rc, Arc)]
    trait Bar<'a, T> {
        fn execute<'b>(
            &'a self,
            arg1: &'b T,
            arg3: &'static str,
        ) -> Result<T, String>;
    }

    println!("yooo");
}


fn main() {}
