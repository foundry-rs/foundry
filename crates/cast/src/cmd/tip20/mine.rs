use crate::{
    cmd::{
        erc20::build_provider_with_signer,
        send::{cast_send, cast_send_with_access_key},
    },
    tx::{SendTxOpts, TxParams},
};
use alloy_primitives::{Address, B256, keccak256};
use alloy_signer::Signer;
use eyre::Result;
use foundry_cli::utils::{LoadConfig, get_chain};
use foundry_common::provider::ProviderBuilder;
use rand::{RngCore, SeedableRng, rngs::StdRng};
use std::time::{Duration, Instant};
use tempo_alloy::{
    TempoNetwork,
    contracts::precompiles::{ADDRESS_REGISTRY_ADDRESS, IAddressRegistry},
};
use tempo_primitives::{MasterId, TempoAddressExt, UserTag};

const POW_BYTES: usize = 4;

pub(super) struct Output {
    pub(super) salt: B256,
    pub(super) registration_hash: B256,
    pub(super) master_id: MasterId,
    pub(super) zero_tag_virtual_address: Address,
}

pub(super) fn run(
    master: Address,
    salt: Option<B256>,
    threads: Option<usize>,
    seed: Option<B256>,
    no_random: bool,
) -> Result<Output> {
    if !master.is_valid_master() {
        eyre::bail!(
            "invalid master address {master}; see https://docs.tempo.xyz/protocol/tips/tip-1022"
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
        print_output(&output, None)?;
        return Ok(output);
    }

    let mut n_threads = threads.unwrap_or(0);
    if n_threads == 0 {
        n_threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    }

    let mut salt = B256::ZERO;
    if !no_random {
        let mut rng = match seed {
            Some(seed) => StdRng::from_seed(seed.0),
            None => StdRng::from_os_rng(),
        };
        rng.fill_bytes(&mut salt[..]);
    }

    sh_println!("Mining TIP-1022 salt for {master} with {n_threads} threads...")?;

    let timer = Instant::now();
    let output = mine(master, salt, n_threads, POW_BYTES)?;
    print_output(&output, Some(timer.elapsed()))?;
    Ok(output)
}

pub(super) async fn register(
    master: Address,
    salt: B256,
    send_tx: SendTxOpts,
    tx_opts: TxParams,
) -> Result<()> {
    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;
    let signer = signer.ok_or_else(|| {
        eyre::eyre!(
            "--register requires a signer or Tempo keychain identity (for example --private-key or --from)"
        )
    })?;

    let sender =
        tempo_access_key.as_ref().map(|ak| ak.wallet_address).unwrap_or_else(|| signer.address());

    if sender != master {
        eyre::bail!(
            "registration sender mismatch: mined salt is for {master}, but the configured signer would register as {sender}"
        );
    }

    let config = send_tx.eth.load_config()?;
    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    let mut tx = IAddressRegistry::new(ADDRESS_REGISTRY_ADDRESS, &provider)
        .registerVirtualMaster(salt)
        .into_transaction_request();
    tx_opts.apply::<TempoNetwork>(&mut tx, get_chain(config.chain, &provider).await?.is_legacy());

    sh_println!("Submitting registerVirtualMaster({salt}) on Tempo...")?;

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

fn mine(master: Address, salt: B256, n_threads: usize, pow_bytes: usize) -> Result<Output> {
    let mut packed = [0u8; 52];
    packed[..20].copy_from_slice(master.as_slice());

    crate::cmd::miner::mine_salt(salt, n_threads, move |salt| {
        packed[20..].copy_from_slice(salt.as_slice());
        let registration_hash = keccak256(packed);

        has_pow(&registration_hash, pow_bytes).then(|| {
            let master_id = MasterId::from_slice(&registration_hash[4..8]);
            let zero_tag_virtual_address = Address::new_virtual(master_id, UserTag::ZERO);
            Output { salt, registration_hash, master_id, zero_tag_virtual_address }
        })
    })
    .ok_or_else(|| eyre::eyre!("virtual master mining failed: all threads panicked"))
}

fn derive(master: Address, salt: B256) -> Output {
    let registration_hash = registration_hash(master, salt);
    let master_id = MasterId::from_slice(&registration_hash[4..8]);
    let zero_tag_virtual_address = Address::new_virtual(master_id, UserTag::ZERO);

    Output { salt, registration_hash, master_id, zero_tag_virtual_address }
}

fn registration_hash(master: Address, salt: B256) -> B256 {
    let mut packed = [0u8; 52];
    packed[..20].copy_from_slice(master.as_slice());
    packed[20..].copy_from_slice(salt.as_slice());
    keccak256(packed)
}

fn has_pow(registration_hash: &B256, pow_bytes: usize) -> bool {
    registration_hash[..pow_bytes].iter().all(|byte| *byte == 0)
}

fn print_output(output: &Output, elapsed: Option<Duration>) -> Result<()> {
    let header = if let Some(elapsed) = elapsed {
        format!("Found salt in {elapsed:?}\n")
    } else {
        String::new()
    };

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
    Ok(())
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
        assert_eq!(output.master_id, MasterId::from_slice(&output.registration_hash[4..8]));
        assert_eq!(
            output.zero_tag_virtual_address,
            Address::new_virtual(output.master_id, UserTag::ZERO),
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
        assert_eq!(
            output.zero_tag_virtual_address,
            Address::new_virtual(output.master_id, UserTag::ZERO),
        );
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

    #[test]
    fn rejects_invalid_master_addresses() {
        assert!(!Address::ZERO.is_valid_master());
        assert!(!address!("0x00000000fdfdfdfdfdfdfdfdfdfd000000000001").is_valid_master());
        assert!(!address!("0x20c0000000000000000000000000000000000001").is_valid_master());
    }
}
