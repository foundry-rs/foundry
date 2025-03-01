use std::fmt;

#[derive(Clone)]
pub enum AssemblyVar {
    Memory(String),
    Variable(String),
    Fixed(String),
}

impl AssemblyVar {
    pub fn memory_access(&self, offset: usize) -> Option<AssemblyVar> {
        match self {
            Self::Variable(a) | Self::Fixed(a) => Some(Self::Memory(format!("{}({})", offset, a))),
            _ => None,
        }
    }

    pub fn memory_accesses(&self, range: usize) -> Vec<AssemblyVar> {
        (0..range)
            .map(|i| {
                let offset = i * 8;
                self.memory_access(offset).unwrap()
            })
            .collect()
    }
}

impl fmt::Display for AssemblyVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Self::Variable(a) | Self::Fixed(a) | Self::Memory(a) => write!(f, "{}", a),
        }
    }
}

impl<'a> From<Declaration<'a>> for AssemblyVar {
    fn from(other: Declaration<'a>) -> Self {
        Self::Variable(format!("{{{}}}", other.name))
    }
}

impl<'a> From<Register<'a>> for AssemblyVar {
    fn from(other: Register<'a>) -> Self {
        Self::Fixed(format!("%{}", other.0))
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Register<'a>(pub &'a str);
impl fmt::Display for Register<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "\"{}\"", self.0)
    }
}

#[derive(Copy, Clone)]
pub struct Declaration<'a> {
    /// Name of the assembly template variable declared by `self`.
    pub name: &'a str,
    /// Rust expression whose value is declared in `self`.
    pub expr: &'a str,
}

impl fmt::Display for Declaration<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{} = in(reg) {},", self.name, self.expr)
    }
}
