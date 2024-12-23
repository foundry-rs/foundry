pub mod qa;

use solar_ast::ast::Span;

pub struct ForgeLint {
    //TODO: settings
}

impl ForgeLint {}

macro_rules! declare_lints {
    ($($name:ident),* $(,)?) => {
        $(
            pub struct $name {
                pub items: Vec<Span>,
            }

            impl $name {
                pub fn new() -> Self {
                    Self { items: Vec::new() }
                }
            }
        )*
    };
}

declare_lints!(
    //Optimizations
    // Vunlerabilities
    // QA
    VariableCamelCase,
    VariableCapsCase,
    VariablePascalCase,
    FunctionCamelCase,
);
