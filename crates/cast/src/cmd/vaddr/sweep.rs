use crate::{
    cmd::{
        erc20::build_provider_with_signer,
        send::{cast_send, cast_send_with_access_key},
    },
    tx::{SendTxOpts, TxParams},
};
use alloy_primitives::Address;
use alloy_signer::Signer;
use alloy_sol_types::sol;
use eyre::Result;
use foundry_cli::utils::{LoadConfig, get_chain};
use foundry_common::provider::ProviderBuilder;
use tempo_alloy::{
    TempoNetwork,
    contracts::precompiles::{ADDRESS_REGISTRY_ADDRESS, IAddressRegistry},
};

sol! {
    #[sol(rpc)]
    interface ITIP20 {
        function balanceOf(address account) external view returns (uint256);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
    }
}

pub(super) async fn run(
    addr: Address,
    token: Address,
    send_tx: SendTxOpts,
    tx_opts: TxParams,
) -> Result<()> {
    let config = send_tx.eth.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    // Resolve master
    let master: Address = IAddressRegistry::new(ADDRESS_REGISTRY_ADDRESS, &provider)
        .resolveVirtualAddress(addr)
        .call()
        .await?;

    if master.is_zero() {
        eyre::bail!("{addr} is not a registered virtual address");
    }

    // Check balance
    let balance: alloy_primitives::U256 =
        ITIP20::new(token, &provider).balanceOf(addr).call().await?;
    if balance.is_zero() {
        sh_println!("Nothing to sweep: balance of {addr} on {token} is 0")?;
        return Ok(());
    }

    sh_println!("Sweeping {balance} from {addr} → {master} on token {token}...")?;

    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;
    let signer = signer
        .ok_or_else(|| eyre::eyre!("cast vaddr sweep requires a signer (the master address)"))?;

    let sender =
        tempo_access_key.as_ref().map(|ak| ak.wallet_address).unwrap_or_else(|| signer.address());

    if sender != master {
        eyre::bail!(
            "signer mismatch: virtual address master is {master}, but the configured signer is {sender}"
        );
    }

    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);

    let mut tx = ITIP20::new(token, &provider)
        .transferFrom(addr, master, balance)
        .into_transaction_request();
    tx_opts.apply::<TempoNetwork>(&mut tx, get_chain(config.chain, &provider).await?.is_legacy());

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
