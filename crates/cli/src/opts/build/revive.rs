use clap::Parser;
use foundry_config::{revive::ResolcConfig, SolcReq};
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize, Parser)]
#[clap(next_help_heading = "Resolc configuration")]
/// Compiler options for resolc
pub struct ResolcOpts {
    #[arg(
        value_name = "RESOLC_COMPILE",
        help = "Enable compiling with resolc",
        long = "resolc-compile",
        visible_alias = "resolc",
        action = clap::ArgAction::SetTrue
    )]
    pub resolc_compile: Option<bool>,

    /// Enable PVM mode at startup (independent of compilation)
    #[arg(
        long = "resolc-startup",
        help = "Enable PVM mode at startup",
        value_name = "RESOLC_STARTUP",
        action = clap::ArgAction::SetTrue
    )]
    pub resolc_startup: Option<bool>,

    /// Specify the resolc version, or a path to a local resolc, to build with.
    ///
    /// Valid values follow the SemVer format `x.y.z-dev.n`, `resolc:x.y.z-dev.n` or
    /// `path/to/resolc`.
    #[arg(
        long = "use-resolc",
        help = "Use resolc version",
        alias = "resolc-compiler-version",
        value_name = "RESOLC_VERSION"
    )]
    #[serde(skip)]
    pub use_resolc: Option<String>,

    /// Set the LLVM optimization parameter `-O[0 | 1 | 2 | 3 | s | z]`.
    /// Use `3` for best performance and `z` for minimal size.
    #[arg(
        short = 'O',
        long = "resolc-optimizer-mode",
        help = "Set the resolc optimization mode `-O[0 | 1 | 2 | 3 | s | z]`",
        visible_alias = "resolc-optimization",
        value_name = "LEVEL"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimizer_mode: Option<String>,

    /// The emulated EVM linear heap memory static buffer size in bytes.
    #[arg(long = "heap-size", help = "Set the contracts heap size in bytes", value_name = "SIZE")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heap_size: Option<u32>,

    /// The contracts total stack size in bytes.
    #[arg(
        long = "stack-size",
        help = "Set the contracts total stack size in bytes",
        value_name = "SIZE"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_size: Option<u32>,

    /// Generate source based debug information in the output code file.
    #[arg(
        long = "debug-info",
        help = "Generate source based debug information in the output code file",
        action = clap::ArgAction::SetTrue
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_information: Option<bool>,
}

impl ResolcOpts {
    pub(crate) fn apply_overrides(&self, mut resolc: ResolcConfig) -> ResolcConfig {
        macro_rules! set_if_some {
            ($src:expr, $dst:expr) => {
                if let Some(src) = $src {
                    $dst = src.into();
                }
            };
        }

        set_if_some!(
            self.resolc_compile.and_then(|v| if v { Some(true) } else { None }),
            resolc.resolc_compile
        );
        set_if_some!(
            self.resolc_startup.and_then(|v| if v { Some(true) } else { None }),
            resolc.resolc_startup
        );
        set_if_some!(
            self.use_resolc.as_ref().map(|v| SolcReq::from(v.trim_start_matches("resolc:"))),
            resolc.resolc
        );
        set_if_some!(
            self.optimizer_mode.as_ref().and_then(|mode| mode.parse::<char>().ok()),
            resolc.optimizer_mode
        );
        set_if_some!(self.heap_size, resolc.heap_size);
        set_if_some!(self.stack_size, resolc.stack_size);
        set_if_some!(
            self.debug_information.and_then(|v| if v { Some(true) } else { None }),
            resolc.debug_information
        );

        resolc
    }
}
