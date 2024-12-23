use solar_ast::Span;

pub mod qa;

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
