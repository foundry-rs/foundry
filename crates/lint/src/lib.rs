pub mod qa;

use solar_ast::ast::Span;

pub struct ForgeLint {
    pub lints: Vec<Lint>,
}

impl ForgeLint {}

macro_rules! declare_lints {
    ($($name:ident),* $(,)?) => {
        #[derive(Debug)]
        pub enum Lint {
            $(
                $name($name),
            )*
        }

        $(
            #[derive(Debug)]
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

// TODO: update macro to include descriptions. Group by opts, vulns, qa
declare_lints!(
    //Optimizations
    // Vunlerabilities
    // QA
    VariableCamelCase,
    VariableCapsCase,
    VariablePascalCase,
    FunctionCamelCase,
);
