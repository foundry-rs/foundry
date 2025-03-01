//! Configuration options

use crate::SysResult;
use crate::raw::empty;

///Function type to empty clipboard
pub type EmptyFn = fn() -> SysResult<()>;

///Clearing parameter
pub trait Clearing {
    ///Empty behavior definition
    const EMPTY_FN: EmptyFn;
}

#[derive(Copy, Clone)]
///Performs no clearing of clipboard
pub struct NoClear;

fn noop() -> SysResult<()> {
    Ok(())
}

impl Clearing for NoClear {
    const EMPTY_FN: EmptyFn = noop;
}

#[derive(Copy, Clone)]
///Performs clearing of clipboard before pasting
pub struct DoClear;

impl Clearing for DoClear {
    const EMPTY_FN: EmptyFn = empty;
}
