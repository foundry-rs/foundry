use crate::tx::{SendTxOpts, TxParams};
use alloy_primitives::{Address, B256};
use clap::Parser;
use foundry_cli::opts::RpcOpts;

mod create;
mod resolve;
mod watch;

/// TIP-1022 virtual address registry operations (Tempo).
///
/// Virtual addresses are deterministic 20-byte aliases (masterId || VIRTUAL_MAGIC || userTag)
/// that auto-forward TIP-20 deposits to a registered master wallet at the protocol level,
/// with no on-chain sweep transaction required.
///
/// See: <https://docs.tempo.xyz/protocol/tips/tip-1022>
#[derive(Debug, Parser, Clone)]
pub enum VaddrSubcommand {
    /// Mine a TIP-1022 proof-of-work salt, register as a virtual address master, and print
    /// derived virtual addresses for the given owner.
    #[command(visible_alias = "c")]
    Create {
        /// The master (owner) address that will control all virtual addresses under this
        /// registration. Must not be the zero address, a virtual address, or a TIP-20 token.
        #[arg(long, value_name = "ADDRESS")]
        owner: Address,

        /// Use this salt directly instead of mining one. Must satisfy the 32-bit PoW requirement.
        #[arg(long, conflicts_with_all = ["seed", "no_random"], value_name = "HEX")]
        salt: Option<B256>,

        /// Starting user tag for the derived virtual address output (hex-encoded 6 bytes).
        #[arg(long, default_value = "0", value_name = "U64")]
        tag: u64,

        /// Number of virtual addresses to derive and print.
        #[arg(long, default_value = "1", value_name = "N")]
        count: u32,

        /// Number of threads to use for mining. Defaults to number of logical cores.
        #[arg(long, short = 'j', visible_alias = "jobs")]
        threads: Option<usize>,

        /// Seed for the random number generator used to initialize the salt search.
        #[arg(long, value_name = "HEX")]
        seed: Option<B256>,

        /// Start salt search from zero instead of a random value.
        #[arg(long, conflicts_with = "seed")]
        no_random: bool,

        /// Mine and print the salt and derived virtual addresses without submitting the
        /// registerVirtualMaster transaction.
        #[arg(long)]
        no_register: bool,

        #[command(flatten)]
        send_tx: Box<SendTxOpts>,

        #[command(flatten)]
        tx: Box<TxParams>,
    },

    /// Resolve a virtual address to its registered master and decode its components.
    #[command(visible_alias = "r")]
    Resolve {
        /// The virtual address to resolve.
        #[arg(value_name = "ADDRESS")]
        addr: Address,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Watch (tail) incoming TIP-20 transfers to a virtual address.
    #[command(visible_alias = "w")]
    Watch {
        /// The virtual address to monitor.
        #[arg(value_name = "ADDRESS")]
        addr: Address,

        /// Filter on a specific TIP-20 token address. Watches all tokens if omitted.
        #[arg(long, value_name = "ADDRESS")]
        token: Option<Address>,

        /// Block number to start from. Defaults to the current latest block.
        #[arg(long, value_name = "BLOCK")]
        from_block: Option<u64>,

        #[command(flatten)]
        rpc: RpcOpts,
    },
}

impl VaddrSubcommand {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            Self::Create {
                owner,
                salt,
                tag,
                count,
                threads,
                seed,
                no_random,
                no_register,
                send_tx,
                tx,
            } => {
                create::run(
                    owner,
                    salt,
                    tag,
                    count,
                    threads,
                    seed,
                    no_random,
                    no_register,
                    *send_tx,
                    *tx,
                )
                .await?
            }
            Self::Resolve { addr, rpc } => resolve::run(addr, rpc).await?,
            Self::Watch { addr, token, from_block, rpc } => {
                watch::run(addr, token, from_block, rpc).await?
            }
        }
        Ok(())
    }
}
