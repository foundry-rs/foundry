use std::time::Duration;

use alloy_network::Network;
use clap::Parser;
use eyre::Result;
use serde::Serialize;

use crate::wallet_browser::signer::BrowserSigner;

/// Browser wallet options
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[command(next_help_heading = "Browser wallet options")]
pub struct BrowserWalletOpts {
    /// Use a browser wallet.
    #[arg(long, help_heading = "")]
    pub browser: bool,

    /// Port for the browser wallet server.
    #[arg(long, value_name = "PORT", default_value = "9545", requires = "browser")]
    pub browser_port: u16,

    /// Whether to open the browser for wallet connection.
    #[arg(long, default_value_t = false, requires = "browser")]
    pub browser_disable_open: bool,

    /// Enable development mode for the browser wallet.
    /// This relaxes certain security features for local development.
    ///
    /// **WARNING**: This should only be used in a development environment.
    #[arg(long, hide = true)]
    pub browser_development: bool,
}

impl BrowserWalletOpts {
    pub async fn run<N: Network>(&self) -> Result<Option<BrowserSigner<N>>> {
        Ok(if self.browser {
            Some(
                BrowserSigner::new(
                    self.browser_port,
                    !self.browser_disable_open,
                    Duration::from_secs(300),
                    self.browser_development,
                )
                .await?,
            )
        } else {
            None
        })
    }
}
