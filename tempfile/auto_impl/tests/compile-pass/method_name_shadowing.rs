use auto_impl::auto_impl;

trait AllExt {
    fn foo(&self, _: i32);
}

impl<T> AllExt for T {
    fn foo(&self, _: i32) {}
}

// This will expand to:
//
//     impl<T: Foo> Foo for &T {
//         fn foo(&self, _x: bool) {
//             T::foo(self, _x)
//         }
//     }
//
// With this test we want to make sure, that the call `T::foo` is always
// unambiguous. Luckily, Rust is nice here. And if we only know `T: Foo`, then
// other global functions are not even considered. Having a test for this
// doesn't hurt though.
#[auto_impl(&)]
trait Foo {
    fn foo(&self, _x: bool);
}


fn main() {}
