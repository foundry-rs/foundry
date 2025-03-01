use std::fmt::Display;
use auto_impl::auto_impl;


#[auto_impl(Fn)]
trait Greeter {
    fn greet<T: Display>(&self, name: T);
}


fn main() {}
