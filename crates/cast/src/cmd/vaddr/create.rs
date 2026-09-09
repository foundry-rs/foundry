use crate::{
    cmd::tip20::mine::{self, RegisterMessages},
    tx::{SendTxOpts, TxParams},
};
use alloy_primitives::{Address, B256};
use eyre::Result;
use foundry_cli::json::print_json_success;
use foundry_common::shell;
use serde_json::json;
use tempo_primitives::{TempoAddressExt, UserTag};

const MESSAGES: RegisterMessages = RegisterMessages {
    no_signer: "cast vaddr create requires a signer (for example --private-key or --from)",
    mismatch: "signer mismatch: salt is for",
    submitting: "...",
};

/// Largest 6-byte user tag.
const MAX_USER_TAG: u64 = 0x0000_FFFF_FFFF_FFFF;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    owner: Address,
    salt: Option<B256>,
    tag: u64,
    count: u32,
    threads: Option<usize>,
    seed: Option<B256>,
    no_random: bool,
    no_register: bool,
    send_tx: SendTxOpts,
    tx_opts: TxParams,
) -> Result<()> {
    if count == 0 {
        return Ok(());
    }

    let json = shell::is_json();
    let (output, elapsed) = mine::find_salt(owner, salt, threads, seed, no_random, json, "owner")?;
    if let Some(elapsed) = elapsed
        && !json
    {
        sh_status!("Found salt in {elapsed:?}")?;
    }

    let mut virtual_addresses = Vec::with_capacity(count as usize);
    for i in 0..count {
        let tag_value = tag
            .checked_add(i as u64)
            .filter(|&t| t <= MAX_USER_TAG)
            .ok_or_else(|| eyre::eyre!("tag overflow: tag + count exceeds the 6-byte user tag range (max {MAX_USER_TAG:#x})"))?;
        let user_tag = UserTag::new(tag_value.to_be_bytes()[2..].try_into().unwrap());
        virtual_addresses.push((user_tag, Address::new_virtual(output.master_id, user_tag)));
    }

    let mut payload = json!({
        "salt": format!("{}", output.salt),
        "registration_hash": format!("{}", output.registration_hash),
        "master_id": format!("{}", output.master_id),
        "virtual_addresses": virtual_addresses.iter().map(|(tag, addr)| json!({
            "tag": format!("{tag}"),
            "address": format!("{addr}"),
        })).collect::<Vec<_>>(),
    });

    if !json {
        sh_println!(
            "Salt:              {}\nRegistration hash: {}\nMaster ID:         {}",
            output.salt,
            output.registration_hash,
            output.master_id,
        )?;
        sh_println!("\nVirtual addresses:")?;
        for (tag, vaddr) in &virtual_addresses {
            sh_println!("  tag={tag}  {vaddr}")?;
        }
    }

    if !no_register {
        let tx_hash =
            mine::register_virtual_master(owner, output.salt, send_tx, tx_opts, json, &MESSAGES)
                .await?;
        payload["registration_tx_hash"] = json!(format!("{tx_hash:#x}"));
    }
    if json {
        print_json_success(payload)?;
    }
    Ok(())
}
