use crate::{
    cmd::{
        erc20::build_provider_with_signer,
        send::{cast_send, cast_send_with_access_key},
        tip20::mine,
    },
    tempo,
    tx::{CastTxSender, SendTxOpts, TxParams},
};
use alloy_network::Network;
use alloy_primitives::{Address, B256};
use alloy_provider::Provider;
use alloy_signer::Signer;
use eyre::Result;
use foundry_cli::{
    json::print_json_success,
    utils::{LoadConfig, get_chain},
};
use foundry_common::{
    FoundryTransactionBuilder,
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::ProviderBuilder,
    shell,
    tempo::{maybe_print_fee_token, resolve_and_set_fee_token},
};
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
            sh_status!("Mining TIP-1022 salt for {owner} with {n_threads} threads...")?;
        }
        let timer = Instant::now();
        let output = mine::mine(owner, start_salt, n_threads, POW_BYTES)?;
        if !shell::is_json() {
            sh_status!("Found salt in {:?}", timer.elapsed())?;
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

    let payload = json!({
        "salt": format!("{}", output.salt),
        "registration_hash": format!("{}", output.registration_hash),
        "master_id": format!("{}", output.master_id),
        "virtual_addresses": virtual_addresses.iter().map(|(tag, addr)| json!({
            "tag": format!("{tag}"),
            "address": format!("{addr}"),
        })).collect::<Vec<_>>(),
    });

    if !shell::is_json() {
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
        if shell::is_json() {
            print_json_success(payload)?;
        }
        return Ok(());
    }

    let tx_hash = register(owner, output.salt, send_tx, tx_opts).await?;

    if shell::is_json() {
        let mut payload = payload;
        payload["registration_tx_hash"] = json!(format!("{tx_hash:#x}"));
        print_json_success(payload)?;
    }

    Ok(())
}

async fn register(
    owner: Address,
    salt: B256,
    send_tx: SendTxOpts,
    mut tx_opts: TxParams,
) -> Result<B256> {
    let config = send_tx.eth.load_config()?;
    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    let chain = get_chain(config.chain, &provider).await?;
    tempo::ensure_session_not_browser(&tx_opts.tempo, send_tx.browser.browser)?;
    let (signer, tempo_access_key) =
        tempo::resolve_session_or_wallet_signer(&tx_opts.tempo, &send_tx.eth.wallet, chain.id())
            .await?;
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

    let mut tx = IAddressRegistry::new(ADDRESS_REGISTRY_ADDRESS, &provider)
        .registerVirtualMaster(salt)
        .into_transaction_request();
    let expires_at = tx_opts.tempo.resolve_expires();
    tempo::print_expires(expires_at)?;
    tx_opts.apply::<TempoNetwork>(&mut tx, chain.is_legacy());

    sh_status!("Submitting registerVirtualMaster({salt})...")?;

    if let Some(ref access_key) = tempo_access_key {
        tempo::fill_access_key_transaction(&provider, &mut tx, access_key, chain).await?;
        if shell::is_json() {
            // JSON mode bypasses `cast_send_with_access_key`, so report the selection here.
            resolve_and_set_fee_token(
                (!config.eth_rpc_curl).then_some(&provider),
                Some(chain),
                &mut tx,
                Some(access_key.wallet_address),
            )
            .await?;
            maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), tx.fee_token())
                .await?;
            let raw_tx = tx
                .sign_with_access_key(
                    &provider,
                    &signer,
                    access_key.wallet_address,
                    access_key.key_address,
                    access_key.key_authorization.as_ref(),
                )
                .await?;
            let tx_hash = *provider.send_raw_transaction(&raw_tx).await?.tx_hash();
            wait_for_receipt_if_needed(
                &provider,
                tx_hash,
                send_tx.cast_async,
                send_tx.confirmations,
                timeout,
            )
            .await?;
            Ok(tx_hash)
        } else {
            cast_send_with_access_key(
                &provider,
                tx,
                &signer,
                access_key,
                Some(chain),
                None,
                send_tx.cast_async,
                send_tx.confirmations,
                timeout,
                !config.eth_rpc_curl,
            )
            .await
        }
    } else {
        let provider = build_provider_with_signer::<TempoNetwork>(&send_tx, signer)?;
        if shell::is_json() {
            // JSON mode bypasses `cast_send`, so report the selection here.
            resolve_and_set_fee_token(
                (!config.eth_rpc_curl).then_some(&provider),
                Some(chain),
                &mut tx,
                Some(sender),
            )
            .await?;
            maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), tx.fee_token())
                .await?;
            let cast = CastTxSender::new(&provider);
            if send_tx.sync {
                cast.send_sync(tx).await.map(|(tx_hash, _)| tx_hash)
            } else {
                let pending_tx = cast.send(tx).await?;
                let tx_hash = *pending_tx.inner().tx_hash();
                wait_for_receipt_if_needed(
                    &provider,
                    tx_hash,
                    send_tx.cast_async,
                    send_tx.confirmations,
                    timeout,
                )
                .await?;
                Ok(tx_hash)
            }
        } else {
            cast_send(
                provider,
                tx,
                Some(chain),
                None,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
                !config.eth_rpc_curl,
            )
            .await
        }
    }
}

async fn wait_for_receipt_if_needed<P: Provider<TempoNetwork>>(
    provider: &P,
    tx_hash: B256,
    cast_async: bool,
    confirmations: u64,
    timeout: u64,
) -> Result<()>
where
    <TempoNetwork as Network>::TransactionRequest: FoundryTransactionBuilder<TempoNetwork>,
    <TempoNetwork as Network>::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    if !cast_async {
        CastTxSender::new(provider)
            .receipt(format!("{tx_hash:#x}"), None, confirmations, Some(timeout), false)
            .await?;
    }
    Ok(())
}
