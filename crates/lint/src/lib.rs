pub mod optimization;
pub mod qa;
pub mod vulnerability;

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

#[derive(Clone, Debug)]
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
    // TODO: Add config specifying which lints to run
    pub fn new(input: Vec<PathBuf>) -> Self {
        Self { input, lints: Lint::all() }
    }

    pub fn with_severity(self, severity: Vec<Severity>) -> Self {
        todo!()
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
    ($($name:ident),* $(,)?) => {
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
            }
        )*
    };
}

// TODO: Group by opts, vulns, qa, add description for each lint
declare_lints!(
    //Optimizations
    Keccak256,
    // Vunlerabilities
    DivideBeforeMultiply,
    // QA
    VariableCamelCase,
    VariableCapsCase,
    StructPascalCase,
    FunctionCamelCase,
);
