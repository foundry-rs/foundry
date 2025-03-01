// Make sure that everything compiles even without the prelude. This basically
// forces us to generate full paths for types of the standard/core library.
//
// Note that `no_implicit_prelude` attribute appears to interact strangely
// with Rust's 2018 style modules and extern crates.
#![no_implicit_prelude]

extern crate std;
extern crate auto_impl;


mod outer {
    use crate::{
        auto_impl::auto_impl,
        std::{
            string::String,
            result::Result,
        }
    };

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

    mod inner {
        use crate::{
            auto_impl::auto_impl,
            std::{
                string::String,
                result::Result,
            }
        };

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
    }
}


fn main() {}
