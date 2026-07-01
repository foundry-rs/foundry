use super::*;

mod bool;
mod cx;
pub(super) mod hashcons;
#[path = "expr.rs"]
mod word;

struct NoopModel;

impl SymbolicModelLookup for NoopModel {
    fn value(&self, _name: Symbol) -> Option<U256> {
        None
    }
}

pub(crate) use bool::*;
pub(crate) use cx::*;
pub(crate) use word::*;
