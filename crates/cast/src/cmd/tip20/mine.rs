use crate::{
    cmd::send::cast_send_raw,
    tempo,
    tx::{CastTxSender, SendTxOpts, TxParams, apply_poll_interval, fill_transaction_gas_fees},
};
use alloy_network::EthereumWallet;
use alloy_primitives::{Address, B256, keccak256};
use alloy_signer::Signer;
use eyre::Result;
use foundry_cli::utils::get_chain;
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder};
use rand::{RngCore, SeedableRng, rngs::StdRng};
use std::time::{Duration, Instant};
use tempo_alloy::{
    TempoNetwork,
    contracts::precompiles::{ADDRESS_REGISTRY_ADDRESS, IAddressRegistry},
};
use tempo_primitives::{MasterId, TempoAddressExt, UserTag};

/// Number of leading zero bytes a TIP-1022 registration hash must have.
pub(crate) const POW_BYTES: usize = 4;

pub(crate) struct Output {
    pub(crate) salt: B256,
    pub(crate) registration_hash: B256,
    pub(crate) master_id: MasterId,
    pub(crate) zero_tag_virtual_address: Address,
}

impl Output {
    fn new(salt: B256, registration_hash: B256) -> Self {
        let master_id = MasterId::from_slice(&registration_hash[4..8]);
        let zero_tag_virtual_address = Address::new_virtual(master_id, UserTag::ZERO);
        Self { salt, registration_hash, master_id, zero_tag_virtual_address }
    }
}

/// Command-specific wording for [`register_virtual_master`].
pub(crate) struct RegisterMessages {
    pub(crate) no_signer: &'static str,
    /// Prefix of the sender-mismatch error, completed with the salt owner and actual sender.
    pub(crate) mismatch: &'static str,
    /// Suffix of the `Submitting registerVirtualMaster(..)` status line.
    pub(crate) submitting: &'static str,
}

pub(super) fn run(
    master: Address,
    salt: Option<B256>,
    threads: Option<usize>,
    seed: Option<B256>,
    no_random: bool,
) -> Result<Output> {
    let (output, elapsed) = find_salt(master, salt, threads, seed, no_random, false, "master")?;
    let header = elapsed.map(|elapsed| format!("Found salt in {elapsed:?}\n")).unwrap_or_default();
    sh_println!(
        r#"{header}Salt:              {}
Registration hash: {}
Master ID:         {}
Zero-tag address:  {}"#,
        output.salt,
        output.registration_hash,
        output.master_id,
        output.zero_tag_virtual_address,
    )?;
    Ok(output)
}

/// Validates `master` and returns the registration to submit: `salt` after checking its proof of
/// work, or a freshly mined salt together with the time it took to find it.
pub(crate) fn find_salt(
    master: Address,
    salt: Option<B256>,
    threads: Option<usize>,
    seed: Option<B256>,
    no_random: bool,
    quiet: bool,
    label: &str,
) -> Result<(Output, Option<Duration>)> {
    if !master.is_valid_master() {
        eyre::bail!(
            "invalid {label} address {master}; see https://docs.tempo.xyz/protocol/tips/tip-1022"
        );
    }

    if let Some(salt) = salt {
        let output = derive(master, salt);
        if !has_pow(&output.registration_hash, POW_BYTES) {
            eyre::bail!(
                "provided salt does not satisfy TIP-1022 proof of work: {}",
                output.registration_hash
            );
        }
        return Ok((output, None));
    }

    let n_threads = match threads {
        Some(n) if n > 0 => n,
        _ => std::thread::available_parallelism().map_or(1, |n| n.get()),
    };
    let mut salt = B256::ZERO;
    if !no_random {
        let mut rng = match seed {
            Some(seed) => StdRng::from_seed(seed.0),
            None => StdRng::from_os_rng(),
        };
        rng.fill_bytes(&mut salt[..]);
    }

    if !quiet {
        sh_status!("Mining TIP-1022 salt for {master} with {n_threads} threads...")?;
    }
    let timer = Instant::now();
    let output = mine(master, salt, n_threads, POW_BYTES)?;
    Ok((output, Some(timer.elapsed())))
}

/// Submits `registerVirtualMaster(salt)` from `master` and returns the transaction hash.
///
/// With `quiet`, neither the transaction hash nor the receipt is printed, but the receipt is still
/// awaited unless `--async` was passed.
pub(crate) async fn register_virtual_master(
    master: Address,
    salt: B256,
    send_tx: SendTxOpts,
    mut tx_opts: TxParams,
    quiet: bool,
    msgs: &RegisterMessages,
) -> Result<B256> {
    let (config, provider) = tempo::tempo_provider(&send_tx.eth.rpc)?;
    apply_poll_interval(&provider, send_tx.poll_interval);
    let chain = get_chain(config.chain, &provider).await?;
    tempo::ensure_session_not_browser(&tx_opts.tempo, send_tx.browser.browser)?;
    let (signer, access_key) =
        tempo::resolve_session_or_wallet_signer(&tx_opts.tempo, &send_tx.eth.wallet, chain.id())
            .await?;
    let sender = match (&access_key, &signer) {
        (Some(wallet), _) => wallet.account(),
        (None, Some(signer)) => signer.address(),
        (None, None) => eyre::bail!("{}", msgs.no_signer),
    };
    if sender != master {
        eyre::bail!(
            "{} {master}, but the configured signer would register as {sender}",
            msgs.mismatch
        );
    }

    let mut tx = IAddressRegistry::new(ADDRESS_REGISTRY_ADDRESS, &provider)
        .registerVirtualMaster(salt)
        .into_transaction_request();
    tempo::print_expires(tx_opts.tempo.resolve_expires())?;
    tx_opts.apply::<TempoNetwork>(&mut tx, chain.is_legacy());
    sh_status!("Submitting registerVirtualMaster({salt}){}", msgs.submitting)?;

    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let fee_provider = (!config.eth_rpc_curl).then_some(&provider);
    let (tx_hash, receipt) = if let Some(access_key) = &access_key {
        let prepared = tempo::fill_access_key_transaction(
            &provider,
            &mut tx,
            access_key,
            chain,
            config.eip1559_fee_estimate,
        )
        .await?;
        tempo::resolve_and_print_fee_token(fee_provider, Some(chain), &mut tx, Some(sender))
            .await?;
        let raw_tx = tx.sign_with_tempo_wallet(&prepared).await?;
        cast_send_raw(&provider, &raw_tx, send_tx.sync).await?
    } else {
        // The wallet provider fills the nonce and gas limit; only the fees are filled here.
        let wallet = EthereumWallet::from(signer.expect("sender was derived from the signer"));
        let signer_provider =
            ProviderBuilder::<TempoNetwork>::from_config(&config)?.build_with_wallet(wallet)?;
        fill_transaction_gas_fees(
            &signer_provider,
            &mut tx,
            chain.is_legacy(),
            false,
            config.eip1559_fee_estimate,
        )
        .await?;
        tempo::resolve_and_print_fee_token(fee_provider, Some(chain), &mut tx, Some(sender))
            .await?;
        let cast = CastTxSender::new(&signer_provider);
        if send_tx.sync {
            let (tx_hash, receipt) = cast.send_sync(tx).await?;
            (tx_hash, Some(receipt))
        } else {
            (*cast.send(tx).await?.inner().tx_hash(), None)
        }
    };

    let cast = CastTxSender::new(&provider);
    match receipt {
        Some(receipt) if !quiet => sh_println!("{receipt}")?,
        Some(_) => {}
        None if quiet => {
            if !send_tx.cast_async {
                let hash = format!("{tx_hash:#x}");
                cast.receipt(hash, None, send_tx.confirmations, Some(timeout), false).await?;
            }
        }
        None => {
            cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout)
                .await?
        }
    }
    Ok(tx_hash)
}

pub(crate) fn mine(
    master: Address,
    salt: B256,
    n_threads: usize,
    pow_bytes: usize,
) -> Result<Output> {
    let mut packed = [0u8; 52];
    packed[..20].copy_from_slice(master.as_slice());

    crate::cmd::miner::mine_salt(salt, n_threads, move |salt| {
        packed[20..].copy_from_slice(salt.as_slice());
        let registration_hash = keccak256(packed);
        has_pow(&registration_hash, pow_bytes).then(|| Output::new(salt, registration_hash))
    })
    .ok_or_else(|| eyre::eyre!("virtual master mining failed: all threads panicked"))
}

pub(crate) fn derive(master: Address, salt: B256) -> Output {
    let mut packed = [0u8; 52];
    packed[..20].copy_from_slice(master.as_slice());
    packed[20..].copy_from_slice(salt.as_slice());
    Output::new(salt, keccak256(packed))
}

pub(crate) fn has_pow(registration_hash: &B256, pow_bytes: usize) -> bool {
    registration_hash[..pow_bytes].iter().all(|byte| *byte == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256};

    #[test]
    fn derives_master_id_and_zero_tag_address() {
        let master = address!("0x1234567890123456789012345678901234567890");
        let salt = b256!("0x0000000000000000000000000000000000000000000000000000000000000001");
        let output = derive(master, salt);

        assert_eq!(
            output.registration_hash,
            b256!("0x661db5481211842e0330ea3e4cf0b4e7e5abd2314161ce16e9a99e7460480f21"),
        );
        assert_eq!(output.master_id, MasterId::from([0x12, 0x11, 0x84, 0x2e]));
        assert_eq!(
            output.zero_tag_virtual_address,
            address!("0x1211842efdfdfdfdfdfdfdfdfdfd000000000000"),
        );
    }

    #[test]
    fn mines_pow_with_reduced_difficulty() -> Result<()> {
        let master = address!("0x1234567890123456789012345678901234567890");
        let output = mine(master, B256::ZERO, 1, 1)?;

        assert_eq!(
            output.salt,
            b256!("0x000000000000000000000000000000000000000000000000f301000000000000"),
        );
        assert_eq!(output.registration_hash[0], 0);
        assert_eq!(output.master_id, MasterId::from_slice(&output.registration_hash[4..8]));
        Ok(())
    }

    #[test]
    fn has_pow_checks_leading_zero_bytes() {
        let mut hash = B256::ZERO;
        assert!(has_pow(&hash, 4));
        assert!(has_pow(&hash, 0));

        hash[3] = 1;
        assert!(!has_pow(&hash, 4));
        assert!(has_pow(&hash, 3));
        assert!(has_pow(&hash, 0));
    }
}
