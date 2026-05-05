use crate::{
    cmd::{
        erc20::build_provider_with_signer,
        send::{cast_send, cast_send_with_access_key},
        tip20::mine,
    },
    tx::{SendTxOpts, TxParams},
};
use alloy_primitives::{Address, B256};
use alloy_signer::Signer;
use eyre::Result;
use foundry_cli::utils::{LoadConfig, get_chain};
use foundry_common::{provider::ProviderBuilder, shell};
use rand::{RngCore, SeedableRng, rngs::StdRng};
use serde_json::json;
use std::time::Instant;
use tempo_alloy::{
    TempoNetwork,
    contracts::precompiles::{ADDRESS_REGISTRY_ADDRESS, IAddressRegistry},
};
use tempo_primitives::{TempoAddressExt, UserTag};

const POW_BYTES: usize = 4;

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
        // no virtual addresses to compute
        return Ok(());
    }

    if !owner.is_valid_master() {
        eyre::bail!(
            "invalid owner address {owner}; see https://docs.tempo.xyz/protocol/tips/tip-1022"
        );
    }

    let output = if let Some(salt) = salt {
        let output = mine::derive(owner, salt);
        if !mine::has_pow(&output.registration_hash, POW_BYTES) {
            eyre::bail!(
                "provided salt does not satisfy TIP-1022 proof of work: {}",
                output.registration_hash
            );
        }
        output
    } else {
        let mut n_threads = threads.unwrap_or(0);
        if n_threads == 0 {
            n_threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        }

        let mut start_salt = B256::ZERO;
        if !no_random {
            let mut rng = match seed {
                Some(seed) => StdRng::from_seed(seed.0),
                None => StdRng::from_os_rng(),
            };
            rng.fill_bytes(&mut start_salt[..]);
        }

        if !shell::is_json() {
            sh_println!("Mining TIP-1022 salt for {owner} with {n_threads} threads...")?;
        }
        let timer = Instant::now();
        let output = mine::mine(owner, start_salt, n_threads, POW_BYTES)?;
        if !shell::is_json() {
            sh_println!("Found salt in {:?}", timer.elapsed())?;
        }
        output
    };

    const MAX_USER_TAG: u64 = 0x0000_FFFF_FFFF_FFFF;
    let mut virtual_addresses = Vec::with_capacity(count as usize);
    for i in 0..count {
        let tag_value = tag
            .checked_add(i as u64)
            .filter(|&t| t <= MAX_USER_TAG)
            .ok_or_else(|| eyre::eyre!("tag overflow: tag + count exceeds the 6-byte user tag range (max {MAX_USER_TAG:#x})"))?;
        let raw = tag_value.to_be_bytes();
        let user_tag = UserTag::new(raw[2..].try_into().expect("slice is 6 bytes"));
        let vaddr = Address::new_virtual(output.master_id, user_tag);
        virtual_addresses.push((user_tag, vaddr));
    }

    if shell::is_json() {
        sh_println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "salt": format!("{}", output.salt),
                "registration_hash": format!("{}", output.registration_hash),
                "master_id": format!("{}", output.master_id),
                "virtual_addresses": virtual_addresses.iter().map(|(tag, addr)| json!({
                    "tag": format!("{tag}"),
                    "address": format!("{addr}"),
                })).collect::<Vec<_>>(),
            }))?
        )?;
    } else {
        sh_println!(
            "Salt:              {}
Registration hash: {}
Master ID:         {}",
            output.salt,
            output.registration_hash,
            output.master_id,
        )?;
        sh_println!("\nVirtual addresses:")?;
        for (tag, vaddr) in &virtual_addresses {
            sh_println!("  tag={tag}  {vaddr}")?;
        }
    }

    if no_register {
        return Ok(());
    }

    register(owner, output.salt, send_tx, tx_opts).await
}

async fn register(
    owner: Address,
    salt: B256,
    send_tx: SendTxOpts,
    tx_opts: TxParams,
) -> Result<()> {
    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;
    let signer = signer.ok_or_else(|| {
        eyre::eyre!("cast vaddr create requires a signer (for example --private-key or --from)")
    })?;

    let sender =
        tempo_access_key.as_ref().map(|ak| ak.wallet_address).unwrap_or_else(|| signer.address());

    if sender != owner {
        eyre::bail!(
            "signer mismatch: salt is for {owner}, but the configured signer would register as {sender}"
        );
    }

    let config = send_tx.eth.load_config()?;
    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    let mut tx = IAddressRegistry::new(ADDRESS_REGISTRY_ADDRESS, &provider)
        .registerVirtualMaster(salt)
        .into_transaction_request();
    tx_opts.apply::<TempoNetwork>(&mut tx, get_chain(config.chain, &provider).await?.is_legacy());

    sh_println!("Submitting registerVirtualMaster({salt})...")?;

    if let Some(ref access_key) = tempo_access_key {
        cast_send_with_access_key(
            &provider,
            tx,
            &signer,
            access_key,
            send_tx.cast_async,
            send_tx.confirmations,
            timeout,
        )
        .await?;
    } else {
        let provider = build_provider_with_signer::<TempoNetwork>(&send_tx, signer)?;
        cast_send(provider, tx, send_tx.cast_async, send_tx.sync, send_tx.confirmations, timeout)
            .await?;
    }

    Ok(())
}
