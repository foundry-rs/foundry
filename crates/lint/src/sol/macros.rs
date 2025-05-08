/// Macro for defining lints and relevant metadata for the Solidity linter.
///
/// # Parameters
///
/// Each lint requires the following input fields:
/// - `$id`: Identitifier of the generated `SolLint` constant.
/// - `$severity`: The `Severity` of the lint (e.g. `High`, `Med`, `Low`, `Info`, `Gas`).
/// - `$str_id`: A unique identifier used to reference a specific lint during configuration.
/// - `$desc`: A short description of the lint.
/// - `$help` (optional): Link to additional information about the lint or best practices.
#[macro_export]
macro_rules! declare_forge_lint {
    ($id:ident, $severity:expr, $str_id:expr, $desc:expr, $help:expr) => {
        // Declare the static `Lint` metadata
        pub static $id: SolLint = SolLint {
            id: $str_id,
            severity: $severity,
            description: $desc,
            help: if $help.is_empty() { None } else { Some($help) },
        };
    };

    ($id:ident, $severity:expr, $str_id:expr, $desc:expr) => {
        $crate::declare_forge_lint!($id, $severity, $str_id, $desc, "");
    };
}

/// Registers Solidity linter passes with their corresponding `SolLint`.
///
/// # Parameters
///
/// - `$pass_id`: Identitifier of the generated struct that will implement the pass trait.
/// - (`$lint`): tuple with `SolLint` constants that should be evaluated on every input that pass.
///
/// # Outputs
///
/// - Structs for each linting pass (which should manually implement `EarlyLintPass`)
/// - `const REGISTERED_LINTS` containing all registered lint objects
/// - `const LINT_PASSES` mapping each lint to its corresponding pass
#[macro_export]
macro_rules! register_lints {
    ( $( ($pass_id:ident, ($($lint:expr),+ $(,)?)) ),* $(,)? ) => {
        // Declare the structs that will implement the pass trait
        $(
            #[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
            pub struct $pass_id;

            impl $pass_id {
                pub fn as_lint_pass<'a>() -> Box<dyn EarlyLintPass<'a>> {
                    Box::new(Self::default())
                }
            }
        )*

        // Expose array constants
        pub const REGISTERED_LINTS: &[SolLint] = &[$( $($lint,) + )*];
        pub const LINT_PASSES: &[(SolLint, fn() -> Box<dyn EarlyLintPass<'static>>)] = &[
            $( $( ($lint, || Box::new($pass_id::default())), )+ )*
        ];

        // Helper function to create lint passes with the required lifetime
        pub fn create_lint_passes<'a>() -> Vec<(Box<dyn EarlyLintPass<'a>>, SolLint)>
        {
            vec![ $( $(($pass_id::as_lint_pass(), $lint), )+ )* ]
        }
    };
}
