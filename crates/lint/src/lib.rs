pub mod gas;
pub mod info;
pub mod med;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use solar_ast::{
    ast::{self, SourceUnit, Span},
    interface::{ColorChoice, Session},
    visit::Visit,
};

#[derive(Clone, Debug)]
pub enum OutputFormat {
    Json,
    Markdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Severity {
    High,
    Med,
    Low,
    Info,
    Gas,
}

pub struct Linter {
    pub input: Vec<PathBuf>,
    pub lints: Vec<Lint>,
    pub description: bool,
}

impl Linter {
    pub fn new(input: Vec<PathBuf>) -> Self {
        Self { input, lints: Lint::all(), description: false }
    }

    pub fn with_severity(mut self, severity: Option<Vec<Severity>>) -> Self {
        if let Some(severity) = severity {
            self.lints.retain(|lint| severity.contains(&lint.severity()));
        }
        self
    }

    pub fn with_description(mut self, description: bool) -> Self {
        self.description = description;
        self
    }

    pub fn lint(self) {
        // Create a new session with a buffer emitter.
        // This is required to capture the emitted diagnostics and to return them at the
        // end.
        let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();

        let mut findings = HashMap::new();

        // Enter the context and parse the file.
        let _ = sess.enter(|| -> solar_interface::Result<()> {
            // Set up the parser.
            let arena = ast::Arena::new();

            for file in &self.input {
                let lints = self.lints.clone();

                let mut parser = solar_parse::Parser::from_file(&sess, &arena, file)
                    .expect("Failed to create parser");
                let ast = parser.parse_file().map_err(|e| e.emit()).expect("Failed to parse file");

                // Run all lints on the parsed AST
                for mut lint in lints {
                    let results = lint.lint(&ast);
                    findings.entry(lint.clone()).or_insert_with(Vec::new).extend(results);
                }
            }

            // TODO: Output the findings

            Ok(())
        });
    }
}

macro_rules! declare_lints {
    ($(($name:ident, $severity:expr, $lint_name:expr, $description:expr)),* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum Lint {
            $(
                $name($name),
            )*
        }

        impl Lint {
            pub fn all() -> Vec<Self> {
                vec![
                    $(
                        Lint::$name($name::new()),
                    )*
                ]
            }

            pub fn severity(&self) -> Severity {
                match self {
                    $(
                        Lint::$name(_) => $severity,
                    )*
                }
            }

            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Lint::$name(_) => $lint_name,
                    )*
                }
            }

            pub fn description(&self) -> &'static str {
                match self {
                    $(
                        Lint::$name(_) => $description,
                    )*
                }
            }


            /// Lint a source unit and return the findings
            pub fn lint<'ast>(&mut self, source_unit: &SourceUnit<'ast>) -> Vec<Span> {
                match self {
                    $(
                        Lint::$name(lint) => {
                            lint.visit_source_unit(source_unit);
                            lint.items.clone()
                        },
                    )*
                }
            }
        }

        impl<'ast> Visit<'ast> for Lint {
            fn visit_source_unit(&mut self, source_unit: &SourceUnit<'ast>) {
                match self {
                    $(
                        Lint::$name(lint) => lint.visit_source_unit(source_unit),
                    )*
                }
            }
        }

        $(
            #[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
            pub struct $name {
                pub items: Vec<Span>,
            }

            impl $name {
                pub fn new() -> Self {
                    Self { items: Vec::new() }
                }

                /// Returns the severity of the lint
                pub fn severity() -> Severity {
                    $severity
                }

                /// Returns the name of the lint
                pub fn name() -> &'static str {
                    $lint_name
                }

                /// Returns the description of the lint
                pub fn description() -> &'static str {
                    $description
                }
            }
        )*
    };
}

declare_lints!(
    // Gas Optimizations
    (Keccak256, Severity::Gas, "Keccak256", "TODO:"),
    //High
    // Med
    (DivideBeforeMultiply, Severity::Med, "Divide Before Multiply", "TODO:"),
    // Low
    // Info
    (VariableCamelCase, Severity::Info, "Variable Camel Case", "TODO:"),
    (VariableCapsCase, Severity::Info, "Variable Caps Case", "TODO:"),
    (StructPascalCase, Severity::Info, "Struct Pascal Case", "TODO:"),
    (FunctionCamelCase, Severity::Info, "Function Camel Case", "TODO:")
);
