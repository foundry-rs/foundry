#![no_std]
#![allow(dead_code)]

use auto_impl::auto_impl;

mod core {}

mod alloc {}

struct Box;
struct Rc;
struct Arc;
struct Fn;
struct FnMut;

#[auto_impl(&, &mut, Box, Rc, Arc)]
trait Test {}

#[auto_impl(Fn)]
trait TestFn {
    fn test(&self);
}

#[auto_impl(FnMut)]
trait TestFnMut {
    fn test(&mut self);
}
