use auto_impl::auto_impl;

trait Supi<'a, T> {
    fn supi(&self);
}

#[auto_impl(Box, &)]
trait Foo<T, U>: Supi<'static, U>
where
    Self: Send
{
    fn foo(&self) -> i32 {
        self.supi();
        3
    }

    fn bar(&self);
}


fn main() {}
