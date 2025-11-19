use std::{
    io::{self, BufRead, Write},
    str::FromStr,
};

use crate::{format_uint_exp, tx::signing_provider};
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_primitives::U256;
use alloy_sol_types::sol;
use clap::Parser;
use foundry_cli::{
    opts::RpcOpts,
    utils::{LoadConfig, get_provider},
};
use foundry_wallets::WalletOpts;

#[doc(hidden)]
pub use foundry_config::utils::*;

/// Simple confirmation prompt that works in both interactive and non-interactive environments.
///
/// In interactive mode (TTY), prompts the user for y/n confirmation.
/// In non-interactive mode (pipe, tests, CI/CD), reads from stdin.
///
/// The prompt is written to stderr to avoid mixing with command output.
fn confirm_prompt(prompt: &str) -> eyre::Result<bool> {
    // Print the prompt to stderr (so it doesn't mix with stdout output)
    sh_eprint!("{prompt} [y/n] ")?;
    io::stderr().flush()?;

    // Read a line from stdin
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;

    // Check if user confirmed
    let answer = input.trim().to_lowercase();
    Ok(answer == "y" || answer == "yes")
}

/// Formats a token amount in human-readable form.
///
/// Fetches token metadata (symbol and decimals) and formats the amount accordingly.
/// Falls back to raw amount display if metadata cannot be fetched.
async fn format_token_amount<P, N>(
    token_contract: &IERC20::IERC20Instance<P, N>,
    amount: U256,
) -> eyre::Result<String>
where
    P: alloy_provider::Provider<N>,
    N: alloy_network::Network,
{
    // Fetch symbol (fallback to "TOKEN" if not available)
    let symbol = token_contract
        .symbol()
        .call()
        .await
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "TOKEN".to_string());

    // Fetch decimals (fallback to raw amount display if not available)
    let formatted_amount = match token_contract.decimals().call().await {
        Ok(decimals) if decimals <= 77 => {
            use alloy_primitives::utils::{ParseUnits, Unit};

            if let Some(unit) = Unit::new(decimals) {
                let formatted = ParseUnits::U256(amount).format_units(unit);

                let trimmed = if let Some(dot_pos) = formatted.find('.') {
                    let fractional = &formatted[dot_pos + 1..];
                    if fractional.chars().all(|c| c == '0') {
                        formatted[..dot_pos].to_string()
                    } else {
                        formatted.trim_end_matches('0').trim_end_matches('.').to_string()
                    }
                } else {
                    formatted
                };
                format!("{trimmed} {symbol}")
            } else {
                sh_warn!("Warning: Could not fetch token decimals. Showing raw amount.")?;
                format!("{amount} {symbol} (raw amount)")
            }
        }
        _ => {
            // Could not fetch decimals, show raw amount
            sh_warn!(
                "Warning: Could not fetch token metadata (decimals/symbol). \
                The address may not be a valid ERC20 token contract."
            )?;
            format!("{amount} {symbol} (raw amount)")
        }
    };

    Ok(formatted_amount)
}

sol! {
    #[sol(rpc)]
    interface IERC20 {
        #[derive(Debug)]
        function name() external view returns (string);
        function symbol() external view returns (string);
        function decimals() external view returns (uint8);
        function totalSupply() external view returns (uint256);
        function balanceOf(address owner) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function approve(address spender, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function mint(address to, uint256 amount) external;
        function burn(uint256 amount) external;
    }
}

/// Interact with ERC20 tokens.
#[derive(Debug, Parser, Clone)]
pub enum Erc20Subcommand {
    /// Query ERC20 token balance.
    #[command(visible_alias = "b")]
    Balance {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The owner to query balance for.
        #[arg(value_parser = NameOrAddress::from_str)]
        owner: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Transfer ERC20 tokens.
    ///
    /// This command will prompt for confirmation before sending the transaction,
    /// displaying the amount in human-readable format (e.g., "100 USDC" instead of raw wei).
    ///
    /// For non-interactive usage (scripts, CI/CD), you can pipe 'yes' to skip the prompt:
    ///
    /// ```sh
    /// yes | cast erc20 transfer <TOKEN> <TO> <AMOUNT> [OPTIONS]
    /// ```
    #[command(visible_alias = "t")]
    Transfer {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The recipient address.
        #[arg(value_parser = NameOrAddress::from_str)]
        to: NameOrAddress,

        /// The amount to transfer (in smallest unit, e.g., wei for 18 decimals).
        amount: String,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        wallet: WalletOpts,
    },

    /// Approve ERC20 token spending.
    ///
    /// This command will prompt for confirmation before sending the transaction,
    /// displaying the amount in human-readable format.
    ///
    /// For non-interactive usage (scripts, CI/CD), you can pipe 'yes' to skip the prompt:
    ///
    /// ```sh
    /// yes | cast erc20 approve <TOKEN> <SPENDER> <AMOUNT> [OPTIONS]
    /// ```
    #[command(visible_alias = "a")]
    Approve {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The spender address.
        #[arg(value_parser = NameOrAddress::from_str)]
        spender: NameOrAddress,

        /// The amount to approve (in smallest unit, e.g., wei for 18 decimals).
        amount: String,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        wallet: WalletOpts,
    },

    /// Query ERC20 token allowance.
    #[command(visible_alias = "al")]
    Allowance {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The owner address.
        #[arg(value_parser = NameOrAddress::from_str)]
        owner: NameOrAddress,

        /// The spender address.
        #[arg(value_parser = NameOrAddress::from_str)]
        spender: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query ERC20 token name.
    #[command(visible_alias = "n")]
    Name {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query ERC20 token symbol.
    #[command(visible_alias = "s")]
    Symbol {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query ERC20 token decimals.
    #[command(visible_alias = "d")]
    Decimals {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query ERC20 token total supply.
    #[command(visible_alias = "ts")]
    TotalSupply {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Mint ERC20 tokens (if the token supports minting).
    #[command(visible_alias = "m")]
    Mint {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The recipient address.
        #[arg(value_parser = NameOrAddress::from_str)]
        to: NameOrAddress,

        /// The amount to mint.
        amount: String,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        wallet: WalletOpts,
    },

    /// Burn ERC20 tokens.
    #[command(visible_alias = "bu")]
    Burn {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The amount to burn.
        amount: String,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        wallet: WalletOpts,
    },
}

impl Erc20Subcommand {
    fn rpc(&self) -> &RpcOpts {
        match self {
            Self::Allowance { rpc, .. } => rpc,
            Self::Approve { rpc, .. } => rpc,
            Self::Balance { rpc, .. } => rpc,
            Self::Transfer { rpc, .. } => rpc,
            Self::Name { rpc, .. } => rpc,
            Self::Symbol { rpc, .. } => rpc,
            Self::Decimals { rpc, .. } => rpc,
            Self::TotalSupply { rpc, .. } => rpc,
            Self::Mint { rpc, .. } => rpc,
            Self::Burn { rpc, .. } => rpc,
        }
    }

    pub async fn run(self) -> eyre::Result<()> {
        let config = self.rpc().load_config()?;
        let provider = get_provider(&config)?;

        match self {
            // Read-only
            Self::Allowance { token, owner, spender, block, .. } => {
                let token = token.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;
                let spender = spender.resolve(&provider).await?;

                let allowance = IERC20::new(token, &provider)
                    .allowance(owner, spender)
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;

                sh_println!("{}", format_uint_exp(allowance))?
            }
            Self::Balance { token, owner, block, .. } => {
                let token = token.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;

                let balance = IERC20::new(token, &provider)
                    .balanceOf(owner)
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", format_uint_exp(balance))?
            }
            Self::Name { token, block, .. } => {
                let token = token.resolve(&provider).await?;

                let name = IERC20::new(token, &provider)
                    .name()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", name)?
            }
            Self::Symbol { token, block, .. } => {
                let token = token.resolve(&provider).await?;

                let symbol = IERC20::new(token, &provider)
                    .symbol()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", symbol)?
            }
            Self::Decimals { token, block, .. } => {
                let token = token.resolve(&provider).await?;

                let decimals = IERC20::new(token, &provider)
                    .decimals()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", decimals)?
            }
            Self::TotalSupply { token, block, .. } => {
                let token = token.resolve(&provider).await?;

                let total_supply = IERC20::new(token, &provider)
                    .totalSupply()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", format_uint_exp(total_supply))?
            }
            // State-changing
            Self::Transfer { token, to, amount, wallet, .. } => {
                let token_addr = token.resolve(&provider).await?;
                let to_addr = to.resolve(&provider).await?;
                let amount = U256::from_str(&amount)?;

                // Format token amount for display
                let token_contract = IERC20::new(token_addr, &provider);
                let formatted_amount = format_token_amount(&token_contract, amount).await?;

                let prompt_msg =
                    format!("Confirm transfer of {formatted_amount} to address {to_addr}");

                if !confirm_prompt(&prompt_msg)? {
                    eyre::bail!("Transfer cancelled by user");
                }

                let provider = signing_provider(wallet, &provider).await?;
                let tx =
                    IERC20::new(token_addr, &provider).transfer(to_addr, amount).send().await?;
                sh_println!("{}", tx.tx_hash())?
            }
            Self::Approve { token, spender, amount, wallet, .. } => {
                let token_addr = token.resolve(&provider).await?;
                let spender_addr = spender.resolve(&provider).await?;
                let amount = U256::from_str(&amount)?;

                // Format token amount for display
                let token_contract = IERC20::new(token_addr, &provider);
                let formatted_amount = format_token_amount(&token_contract, amount).await?;

                let prompt_msg = format!(
                    "Confirm approval for {spender_addr} to spend {formatted_amount} from your account"
                );

                if !confirm_prompt(&prompt_msg)? {
                    eyre::bail!("Approval cancelled by user");
                }

                let provider = signing_provider(wallet, &provider).await?;
                let tx =
                    IERC20::new(token_addr, &provider).approve(spender_addr, amount).send().await?;
                sh_println!("{}", tx.tx_hash())?
            }
            Self::Mint { token, to, amount, wallet, .. } => {
                let token = token.resolve(&provider).await?;
                let to = to.resolve(&provider).await?;
                let amount = U256::from_str(&amount)?;

                let provider = signing_provider(wallet, &provider).await?;
                let tx = IERC20::new(token, &provider).mint(to, amount).send().await?;
                sh_println!("{}", tx.tx_hash())?
            }
            Self::Burn { token, amount, wallet, .. } => {
                let token = token.resolve(&provider).await?;
                let amount = U256::from_str(&amount)?;

                let provider = signing_provider(wallet, &provider).await?;
                let tx = IERC20::new(token, &provider).burn(amount).send().await?;
                sh_println!("{}", tx.tx_hash())?
            }
        };
        Ok(())
    }
}
