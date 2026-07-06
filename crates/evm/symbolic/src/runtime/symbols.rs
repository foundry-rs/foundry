use super::*;
use inturn::InternerSymbol;
use std::num::NonZeroU32;

pub(crate) type SymbolicVars = IndexSet<Symbol>;

pub(crate) type SymbolicModel = HashMap<Symbol, U256>;

pub(crate) trait SymbolicModelLookup {
    fn value(&self, name: Symbol) -> Option<U256>;

    fn contains_name(&self, name: Symbol) -> bool {
        self.value(name).is_some()
    }
}

impl SymbolicModelLookup for SymbolicModel {
    fn value(&self, name: Symbol) -> Option<U256> {
        self.get(&name).copied()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Symbol(NonZeroU32);

impl Symbol {
    pub(crate) const fn id(&self) -> NonZeroU32 {
        self.0
    }
}

impl InternerSymbol for Symbol {
    fn try_from_usize(id: usize) -> Option<Self> {
        id.checked_add(1).and_then(|id| u32::try_from(id).ok()).and_then(NonZeroU32::new).map(Self)
    }

    fn to_usize(self) -> usize {
        usize::try_from(self.0.get() - 1).expect("symbol id fits usize")
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sym{}", self.id())
    }
}
