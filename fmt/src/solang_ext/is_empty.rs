use solang_parser::pt::*;

/// Describes if a block is empty or not
pub trait IsEmpty {
    fn is_empty(&self) -> bool;
}

impl IsEmpty for Statement {
    fn is_empty(&self) -> bool {
        match self {
            Statement::Block { statements, .. } => statements.is_empty(),
            _ => false,
        }
    }
}

impl IsEmpty for FunctionDefinition {
    fn is_empty(&self) -> bool {
        if let Some(body) = &self.body {
            body.is_empty()
        } else {
            true
        }
    }
}
