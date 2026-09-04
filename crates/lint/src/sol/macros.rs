/// Declares the static metadata of a lint.
///
/// - `$id`: identifier of the generated `SolLint` constant.
/// - `$severity`: the `Severity` of the lint.
/// - `$str_id`: the user-facing lint id used in configuration and diagnostics.
/// - `$desc`: a short description.
///
/// Each lint must have a markdown page at `crates/lint/docs/<str_id>.md`; the `help` URL is
/// derived from `$str_id` and validated by a unit test in `crates/lint/src/sol/mod.rs`.
#[macro_export]
macro_rules! declare_forge_lint {
    ($id:ident, $severity:expr, $str_id:expr, $desc:expr) => {
        pub static $id: SolLint = SolLint {
            id: $str_id,
            severity: $severity,
            description: $desc,
            help: concat!("https://getfoundry.sh/forge/linting/", $str_id),
        };
    };
}

/// Declares the lint modules of a severity group and registers their passes.
///
/// Each entry is `module: (PassStruct, early|late|project, (LINT, ...) [, constructor]), ...;`.
/// The macro declares `mod module;`, glob-imports it, generates the `PassStruct` marker types,
/// the `REGISTERED_LINTS` slice and a `register_lints` function that adds every pass to Solar's
/// registry. A constructor, when given, receives the lint-specific configuration.
#[macro_export]
macro_rules! register_lints {
    (@register $registry:ident, $config:ident, $pass:ident, early $(, $ctor:expr)?) => {
        register_lints!(@call $registry, $config, $pass, register_early_pass $(, $ctor)?);
    };
    (@register $registry:ident, $config:ident, $pass:ident, late $(, $ctor:expr)?) => {
        register_lints!(@call $registry, $config, $pass, register_late_pass $(, $ctor)?);
    };
    (@register $registry:ident, $config:ident, $pass:ident, project $(, $ctor:expr)?) => {
        register_lints!(@call $registry, $config, $pass, register_project_pass $(, $ctor)?);
    };

    (@call $registry:ident, $config:ident, $pass:ident, $method:ident, $ctor:expr) => {{
        let config = std::sync::Arc::clone($config);
        $registry.$method($pass::LINT_IDS, move || ($ctor)(std::sync::Arc::clone(&config)));
    }};
    (@call $registry:ident, $config:ident, $pass:ident, $method:ident) => {
        $registry.$method($pass::LINT_IDS, $pass::default);
    };

    ( $( $module:ident: $( ($pass:ident, $kind:ident, ($($lint:ident),* $(,)?) $(, $ctor:expr)?) ),+ $(,)? ; )* ) => {
        $(
            mod $module;
            use $module::*;

            $(
                #[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
                pub struct $pass;

                impl $pass {
                    const LINT_IDS: &'static [&'static str] = &[$($lint.id),*];
                }
            )+
        )*

        pub const REGISTERED_LINTS: &[SolLint] = &[$($($($lint,)*)+)*];

        pub fn register_lints(
            registry: &mut solar_lint::LintRegistry,
            config: &std::sync::Arc<foundry_config::lint::LintSpecificConfig>,
        ) {
            let _ = config;
            $($( register_lints!(@register registry, config, $pass, $kind $(, $ctor)?); )+)*
        }
    };
}
