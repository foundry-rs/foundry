use super::*;

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

#[cfg(test)]
impl SymbolicModelLookup for BTreeMap<String, U256> {
    fn value(&self, name: Symbol) -> Option<U256> {
        self.get(name.as_str()).copied()
    }
}

type SymbolInterner = inturn::Interner<Symbol, DefaultHashBuilder>;

static SYMBOL_INTERNER: LazyLock<SymbolInterner> =
    LazyLock::new(|| SymbolInterner::with_capacity_and_hasher(1024, DefaultHashBuilder::default()));

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Symbol(NonZeroU32);

impl Symbol {
    pub(crate) fn intern(name: &str) -> Self {
        SYMBOL_INTERNER.intern(name)
    }

    pub(crate) fn as_str(self) -> &'static str {
        SYMBOL_INTERNER.resolve(self)
    }
}

impl inturn::InternerSymbol for Symbol {
    fn try_from_usize(id: usize) -> Option<Self> {
        let id = u32::try_from(id).ok()?.checked_add(1)?;
        NonZeroU32::new(id).map(Self)
    }

    fn to_usize(self) -> usize {
        self.0.get() as usize - 1
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
