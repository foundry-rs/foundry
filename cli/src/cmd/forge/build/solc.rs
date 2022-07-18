use clap::Parser;
use foundry_config::{
    figment::{
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
        Error, Metadata, Profile, Provider,
    },
    Config,
};
use serde::Serialize;

/// Solc-specific build arguments used by multiple subcommands.
#[derive(Debug, Clone, Parser, Serialize, Default)]
pub struct SolcArgs {
    #[clap(help_heading = "COMPILER OPTIONS", help = "Do not auto-detect solc.", long)]
    #[serde(skip)]
    pub no_auto_detect: bool,

    /// Specify the solc version, or a path to a local solc, to build with.
    ///
    /// Valid values are in the format `x.y.z`, `solc:x.y.z` or `path/to/solc`.
    #[clap(help_heading = "COMPILER OPTIONS", value_name = "SOLC_VERSION", long = "use")]
    #[serde(skip)]
    pub use_solc: Option<String>,
}

impl Provider for SolcArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Solc Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let value = Value::serialize(self)?;
        let error = InvalidType(value.to_actual(), "map".into());
        let mut dict = value.into_dict().ok_or(error)?;

        if self.no_auto_detect {
            dict.insert("auto_detect_solc".to_string(), false.into());
        }

        if let Some(ref solc) = self.use_solc {
            dict.insert("solc".to_string(), solc.trim_start_matches("solc:").into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
