//! Smoke tests for the packaged Tempo CLI.
#![cfg(feature = "tempo")]
use alloy_primitives::{Address, B256, U256};
use alloy_provider::Provider;
use alloy_sol_types::{SolValue, sol};
use foundry_common::provider::get_http_provider as http_provider;
use foundry_evm_core::tempo::PATH_USD_ADDRESS as PATH_USD;
use std::{
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tempo_precompiles::{
    ADDRESS_REGISTRY_ADDRESS, RECEIVE_POLICY_GUARD_ADDRESS, TIP20_CHANNEL_RESERVE_ADDRESS,
    receive_policy_guard::{IReceivePolicyGuard, InboundKind},
    tip403_registry::ITIP403Registry,
};
sol! {
    #[sol(rpc)]
    interface IAddressRegistryRpc {
        function isImplicitlyApproved(address target) external view returns (bool);
    }
    #[sol(rpc)]
    interface ITIP20ChannelReserveT5Rpc {
        function domainSeparator() external view returns (bytes32);
    }
}
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn anvil_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_anvil"))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_cli_tempo_t5_hardfork_precompile_smoke() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let port_arg = port.to_string();

    let mut child = ChildGuard(
        Command::new(anvil_binary())
            .args([
                "--network",
                "tempo",
                "--hardfork",
                "tempo:T5",
                "--host",
                "127.0.0.1",
                "--port",
                &port_arg,
                "-q",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn anvil --hardfork tempo:T5"),
    );

    let endpoint = format!("http://127.0.0.1:{port}");
    let provider = http_provider(&endpoint);
    let mut ready = false;
    // The node starts a reth node, which takes longer than anvil's in-memory backend.
    for _ in 0..600 {
        if provider.get_chain_id().await.is_ok() {
            ready = true;
            break;
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            panic!("anvil exited before serving RPC: {status}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ready, "anvil --hardfork tempo:T5 should start serving RPC");

    let registry = IAddressRegistryRpc::new(ADDRESS_REGISTRY_ADDRESS, &provider);
    assert!(registry.isImplicitlyApproved(TIP20_CHANNEL_RESERVE_ADDRESS).call().await.unwrap());

    let reserve = ITIP20ChannelReserveT5Rpc::new(TIP20_CHANNEL_RESERVE_ADDRESS, &provider);
    assert_ne!(reserve.domainSeparator().call().await.unwrap(), B256::ZERO);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_cli_tempo_t6_hardfork_receive_policy_guard_smoke() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let port_arg = port.to_string();

    let mut child = ChildGuard(
        Command::new(anvil_binary())
            .args([
                "--network",
                "tempo",
                "--hardfork",
                "tempo:T6",
                "--host",
                "127.0.0.1",
                "--port",
                &port_arg,
                "-q",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn anvil --hardfork tempo:T6"),
    );

    let endpoint = format!("http://127.0.0.1:{port}");
    let provider = http_provider(&endpoint);
    let mut ready = false;
    // The node starts a reth node, which takes longer than anvil's in-memory backend.
    for _ in 0..600 {
        if provider.get_chain_id().await.is_ok() {
            ready = true;
            break;
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            panic!("anvil exited before serving RPC: {status}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ready, "anvil --hardfork tempo:T6 should start serving RPC");

    let receipt = IReceivePolicyGuard::ClaimReceiptV1::new(
        PATH_USD,
        Address::with_last_byte(2),
        Address::with_last_byte(3),
        Address::with_last_byte(4),
        1,
        1,
        ITIP403Registry::BlockedReason::RECEIVE_POLICY as u8,
        InboundKind::TRANSFER,
        B256::ZERO,
    )
    .abi_encode()
    .into();
    let guard = IReceivePolicyGuard::new(RECEIVE_POLICY_GUARD_ADDRESS, &provider);
    assert_eq!(guard.balanceOf(receipt).call().await.unwrap(), U256::ZERO);
}
