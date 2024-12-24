pub mod gas;
pub mod info;
pub mod med;

use std::path::{Path, PathBuf};

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
}

impl Linter {
    pub fn new(input: Vec<PathBuf>) -> Self {
        Self { input, lints: Lint::all() }
    }

    pub fn with_severity(mut self, severities: Option<Vec<Severity>>) -> Self {
        if let Some(severities) = severities {
            self.lints.retain(|lint| severities.contains(&lint.severity()));
        }
        self
    }

    pub fn lint(self) {
        // Create a new session with a buffer emitter.
        // This is required to capture the emitted diagnostics and to return them at the
        // end.
        let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();

        // Enter the context and parse the file.
        let _ = sess.enter(|| -> solar_interface::Result<()> {
            // Set up the parser.
            let arena = ast::Arena::new();

            let mut parser =
                solar_parse::Parser::from_file(&sess, &arena, &Path::new(&source)).expect("TODO:");

            // Parse the file.
            let ast = parser.parse_file().map_err(|e| e.emit()).expect("TODO:");

            for mut lint in self.lints {
                lint.visit_source_unit(&ast);
            }

            Ok(())
        });
    }
}

macro_rules! declare_lints {
    ($(($name:ident, $severity:expr, $lint_name:expr, $description:expr)),* $(,)?) => {
        #[derive(Debug)]
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
            #[derive(Debug, Default)]
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
