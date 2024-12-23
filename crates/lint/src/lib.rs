pub mod qa;

use std::path::{Path, PathBuf};

use solar_ast::{
    ast::{self, SourceUnit, Span},
    interface::{ColorChoice, Session},
    visit::Visit,
};

#[derive(Debug)]
pub enum Input {
    Stdin(String),
    Paths(Vec<PathBuf>),
}

pub struct ForgeLint {
    pub input: Input,
    pub lints: Vec<Lint>,
}

impl ForgeLint {
    // TODO: Add config specifying which lints to run
    pub fn new(input: Input) -> Self {
        Self { input, lints: Lint::all() }
    }

    pub fn lint(self) {
        match self.input {
            Input::Stdin(source) => {
                // Create a new session with a buffer emitter.
                // This is required to capture the emitted diagnostics and to return them at the
                // end.
                let sess = Session::builder().with_buffer_emitter(ColorChoice::Auto).build();

                // Enter the context and parse the file.
                let _ = sess.enter(|| -> solar_interface::Result<()> {
                    // Set up the parser.
                    let arena = ast::Arena::new();

                    let mut parser =
                        solar_parse::Parser::from_file(&sess, &arena, &Path::new(&source))
                            .expect("TODO:");

                    // Parse the file.
                    let ast = parser.parse_file().map_err(|e| e.emit()).expect("TODO:");

                    for mut lint in self.lints {
                        lint.visit_source_unit(&ast);
                    }

                    Ok(())
                });
            }

            Input::Paths(paths) => {
                if paths.is_empty() {
                    // sh_warn!(
                    //     "Nothing to lint.\n\
                    //      HINT: If you are working outside of the project, \
                    //      try providing paths to your source files: `forge fmt <paths>`"
                    // )?;
                    todo!();
                }

                todo!();
            }
        };
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

// TODO: Group by opts, vulns, qa, add description for each lint
declare_lints!(
    //Optimizations
    // Vunlerabilities
    // QA
    VariableCamelCase,
    VariableCapsCase,
    VariablePascalCase,
    FunctionCamelCase,
);
