use solang_parser::pt::*;
use std::fmt::{self, Display};

macro_rules! operators {
    ($($op:ident $repr:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
        #[repr(u8)]
        pub(crate) enum Operator {
            $($op,)*
        }

        impl Operator {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$op => $repr,)*
                }
            }

            pub fn for_expr(expr: &Expression) -> Option<Self> {
                match expr {
                    $(Expression::$op(..) => Some(Self::$op),)*
                    _ => None
                }
            }

        }
    };
}

// Order denotes the precedence
operators! {
    Add "+",
    Multiply "*",
    Assign "=",
}

impl Display for Operator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
