//! Tempo transaction helpers used by Cast-facing commands.

use eyre::Result;

pub use foundry_common::tempo::{TempoSponsor, TempoSponsorPreview, resolve_tempo_sponsor_signer};

pub(crate) fn print_expires(expires_at: Option<u64>) -> Result<()> {
    if let Some(ts) = expires_at {
        sh_println!("Transaction expires at unix timestamp {ts}")?;
    }
    Ok(())
}
