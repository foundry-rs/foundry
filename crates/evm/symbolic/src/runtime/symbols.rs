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

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Symbol(Arc<str>);

impl Symbol {
    pub(crate) const fn new(name: Arc<str>) -> Self {
        Self(name)
    }

    pub(crate) fn intern(name: &str) -> Self {
        Self(Arc::from(name))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
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
