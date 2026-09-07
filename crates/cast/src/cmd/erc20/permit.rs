//! ERC-2612 signed approvals.

use std::str::FromStr;

use crate::{
    cmd::send::SendTxArgs,
    tempo,
    tx::{SendTxOpts, TxParams},
};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_dyn_abi::TypedData;
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, Network};
use alloy_primitives::{Address, B256, U256, hex};
use alloy_provider::Provider;
use alloy_signer::{Signature, Signer};
use alloy_sol_types::{Eip712Domain, SolCall, sol};
use clap::Args;
use eyre::{Result, WrapErr, ensure};
use foundry_cli::{
    json::{print_json_success, print_scalar},
    utils::{LoadConfig, get_chain, get_provider},
};
use foundry_common::{
    FoundryTransactionBuilder,
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::ProviderBuilder,
    shell,
};
use foundry_config::Config;
use foundry_wallets::WalletSigner;
use serde::Serialize;
use serde_json::json;
use tempo_alloy::TempoNetwork;

sol! {
    #[sol(rpc)]
    interface IERC2612 {
        function name() external view returns (string);
        function nonces(address owner) external view returns (uint256);
        function DOMAIN_SEPARATOR() external view returns (bytes32);
        function eip712Domain() external view returns (
            bytes1 fields, string name, string version, uint256 chainId,
            address verifyingContract, bytes32 salt, uint256[] extensions
        );
    }

    function permit(address owner, address spender, uint256 value, uint256 deadline,
        uint8 v, bytes32 r, bytes32 s) external;

    #[derive(Serialize)]
    struct Permit {
        address owner;
        address spender;
        uint256 value;
        uint256 nonce;
        uint256 deadline;
    }
}

/// Arguments for signing and submitting an ERC-2612 permit.
#[derive(Debug, Clone, Args)]
pub struct PermitArgs {
    /// The ERC-2612 token contract address.
    #[arg(value_parser = NameOrAddress::from_str)]
    token: NameOrAddress,
    /// The spender authorized by the permit.
    #[arg(value_parser = NameOrAddress::from_str)]
    spender: NameOrAddress,
    /// The allowance to set, in raw token units.
    amount: U256,
    /// Absolute Unix timestamp in seconds after which the permit cannot be submitted.
    #[arg(long)]
    deadline: U256,
    /// Override the EIP-712 domain name.
    #[arg(long)]
    domain_name: Option<String>,
    /// Override the EIP-712 domain version (fallback: "1").
    #[arg(long)]
    domain_version: Option<String>,
    /// Submit the permit transaction using the signing wallet.
    #[arg(long)]
    broadcast: bool,
    #[command(flatten)]
    pub(super) send_tx: SendTxOpts,
    #[command(flatten)]
    tx: TxParams,
}

impl PermitArgs {
    pub(super) async fn run(self) -> Result<()> {
        self.ensure_broadcast_options()?;
        ensure!(
            self.tx.tempo.session_id()?.is_none(),
            "Tempo sessions cannot sign ERC-2612 permits"
        );
        let config = self.send_tx.eth.load_config()?;
        let provider = get_provider(&config)?;
        let rpc_chain_id = provider.get_chain_id().await?;
        ensure!(
            config.chain.is_none_or(|chain| chain.id() == rpc_chain_id),
            "Configured chain does not match the RPC chain"
        );
        let rpc_is_tempo = get_chain(config.chain, &provider).await?.is_tempo();
        let (resolved_tempo, signer, access_key) =
            tempo::resolve_transaction_network_and_signer(&self.tx.tempo, &self.send_tx.eth)
                .await?;
        ensure!(
            access_key.is_none(),
            "Tempo access keys cannot sign ERC-2612 permits; use a root account signer"
        );
        let is_tempo = resolved_tempo || (self.send_tx.browser.browser && rpc_is_tempo);
        if is_tempo {
            self.run_generic::<TempoNetwork>(signer, config, rpc_chain_id).await
        } else {
            self.run_generic::<Ethereum>(signer, config, rpc_chain_id).await
        }
    }

    async fn run_generic<N: Network>(
        self,
        pre_resolved_signer: Option<WalletSigner>,
        config: Config,
        rpc_chain_id: u64,
    ) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;
        let token = self.token.resolve(&provider).await?;
        let spender = self.spender.resolve(&provider).await?;
        let chain_id = U256::from(rpc_chain_id);
        let contract = IERC2612::new(token, &provider);
        let separator = contract
            .DOMAIN_SEPARATOR()
            .call()
            .await
            .wrap_err("Could not read ERC-2612 DOMAIN_SEPARATOR()")?;
        let mut domain = match contract.eip712Domain().call().await {
            Ok(domain) => discovered_domain(domain)?,
            Err(_) => Eip712Domain {
                name: Some(
                    match &self.domain_name {
                        Some(name) => name.clone(),
                        None => contract
                            .name()
                            .call()
                            .await
                            .wrap_err("Could not read name(); supply --domain-name")?,
                    }
                    .into(),
                ),
                version: Some("1".into()),
                chain_id: Some(chain_id),
                verifying_contract: Some(token),
                ..Default::default()
            },
        };
        if let Some(name) = self.domain_name {
            domain.name = Some(name.into());
        }
        if let Some(version) = self.domain_version {
            domain.version = Some(version.into());
        }
        validate_domain(&domain, separator, chain_id, token)?;

        let browser = self.send_tx.browser.run::<N>().await?;
        if let Some(browser) = &browser
            && domain.chain_id.is_some()
            && browser.chain_id() != rpc_chain_id
        {
            browser.switch_chain(rpc_chain_id).await?;
        }
        let wallet = if browser.is_none() {
            Some(match pre_resolved_signer {
                Some(signer) => signer,
                None => self.send_tx.eth.wallet.signer().await?,
            })
        } else {
            None
        };
        let owner = browser
            .as_ref()
            .map(|wallet| wallet.address())
            .unwrap_or_else(|| wallet.as_ref().expect("signer resolved").address());
        ensure!(
            self.send_tx.eth.wallet.from.is_none_or(|from| from == owner),
            "--from must match the permit signing wallet"
        );
        let nonce = contract
            .nonces(owner)
            .call()
            .await
            .wrap_err("Could not read ERC-2612 nonces(owner)")?;
        let permit = Permit { owner, spender, value: self.amount, nonce, deadline: self.deadline };
        let typed_data = TypedData::from_struct(&permit, Some(domain));
        let signature = if let Some(browser) = &browser {
            browser.sign_dynamic_typed_data(&typed_data).await?
        } else {
            wallet.as_ref().expect("signer resolved").sign_dynamic_typed_data(&typed_data).await?
        };
        let calldata = permitCall {
            owner,
            spender,
            value: self.amount,
            deadline: self.deadline,
            v: signature.v_byte(),
            r: signature.r().into(),
            s: signature.s().into(),
        }
        .abi_encode();
        if self.broadcast {
            let send = SendTxArgs::contract_call(token.into(), calldata, self.send_tx, self.tx);
            return if let Some(browser) = browser {
                send.run_generic_with_browser::<N>(browser).await
            } else {
                send.run_generic::<N>(wallet, None).await
            };
        }
        if shell::is_json() {
            print_json_success(json!({
                "token": token, "owner": owner, "spender": spender,
                "value": self.amount.to_string(), "nonce": nonce.to_string(),
                "deadline": self.deadline.to_string(),
                "signature": hex::encode_prefixed(signature.as_bytes()),
                "calldata": hex::encode_prefixed(calldata), "typed_data": typed_data,
            }))?;
        } else {
            print_scalar(hex::encode_prefixed(signature.as_bytes()))?;
        }
        Ok(())
    }

    fn ensure_broadcast_options(&self) -> Result<()> {
        let send_options = self.send_tx.cast_async
            || self.send_tx.sync
            || self.send_tx.confirmations != 1
            || self.send_tx.timeout.is_some()
            || self.send_tx.poll_interval.is_some();
        let transaction_options = self.tx.gas_limit.is_some()
            || self.tx.gas_price.is_some()
            || self.tx.priority_gas_price.is_some()
            || self.tx.nonce.is_some()
            || self.tx.tempo.is_tempo()
            || self.tx.tempo.session_id()?.is_some()
            || self.tx.tempo.lanes_file.is_some();
        ensure!(
            self.broadcast || !(send_options || transaction_options),
            "Transaction options require --broadcast"
        );
        Ok(())
    }
}

fn discovered_domain(domain: IERC2612::eip712DomainReturn) -> Result<Eip712Domain> {
    let fields = domain.fields[0];
    ensure!(
        fields & !0x1f == 0 && domain.extensions.is_empty(),
        "Unsupported EIP-712 domain fields or extensions"
    );
    Ok(Eip712Domain {
        name: (fields & 1 != 0).then(|| domain.name.into()),
        version: (fields & 2 != 0).then(|| domain.version.into()),
        chain_id: (fields & 4 != 0).then_some(domain.chainId),
        verifying_contract: (fields & 8 != 0).then_some(domain.verifyingContract),
        salt: (fields & 16 != 0).then_some(domain.salt),
    })
}

fn validate_domain(
    domain: &Eip712Domain,
    separator: B256,
    chain_id: U256,
    token: Address,
) -> Result<()> {
    ensure!(
        domain.chain_id.is_none_or(|id| id == chain_id),
        "EIP-712 domain chain ID does not match the RPC chain"
    );
    ensure!(
        domain.verifying_contract.is_none_or(|address| address == token),
        "EIP-712 domain verifying contract does not match the token"
    );
    ensure!(
        domain.separator() == separator,
        "EIP-712 domain does not match DOMAIN_SEPARATOR(); check --domain-name and --domain-version"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::erc20::Erc20Subcommand;
    use clap::Parser;

    #[test]
    fn permit_requires_explicit_deadline_and_valid_amount() {
        let args = [
            "erc20",
            "permit",
            "0x0000000000000000000000000000000000000001",
            "0x0000000000000000000000000000000000000002",
            "123",
        ];
        assert!(Erc20Subcommand::try_parse_from(args).is_err());
        assert!(
            Erc20Subcommand::try_parse_from(args.into_iter().chain(["--deadline", "4000000000"]))
                .is_ok()
        );
        let mut invalid = args;
        invalid[4] = "-1";
        assert!(
            Erc20Subcommand::try_parse_from(
                invalid.into_iter().chain(["--deadline", "4000000000"])
            )
            .is_err()
        );
    }

    #[test]
    fn permit_requires_broadcast_for_transaction_options() {
        let base = [
            "erc20",
            "permit",
            "0x0000000000000000000000000000000000000001",
            "0x0000000000000000000000000000000000000002",
            "123",
            "--deadline",
            "4000000000",
        ];
        for option in [["--async", ""], ["--nonce", "1"], ["--gas-limit", "21000"]] {
            let args = base.into_iter().chain(option.into_iter().filter(|value| !value.is_empty()));
            let Erc20Subcommand::Permit(args) = Erc20Subcommand::try_parse_from(args).unwrap()
            else {
                unreachable!()
            };
            assert_eq!(
                args.ensure_broadcast_options().unwrap_err().to_string(),
                "Transaction options require --broadcast"
            );
        }

        let args = base.into_iter().chain(["--async", "--broadcast"]);
        let Erc20Subcommand::Permit(args) = Erc20Subcommand::try_parse_from(args).unwrap() else {
            unreachable!()
        };
        assert!(args.ensure_broadcast_options().is_ok());
    }

    #[test]
    fn domain_discovery_respects_fields_and_salt() {
        let domain = discovered_domain(IERC2612::eip712DomainReturn {
            fields: [0x14].into(),
            name: "ignored".into(),
            version: "ignored".into(),
            chainId: U256::from(1),
            verifyingContract: Address::ZERO,
            salt: B256::repeat_byte(42),
            extensions: vec![],
        })
        .unwrap();
        assert_eq!(
            domain,
            Eip712Domain {
                chain_id: Some(U256::from(1)),
                salt: Some(B256::repeat_byte(42)),
                ..Default::default()
            }
        );
    }

    #[test]
    fn domain_discovery_rejects_extensions_and_unknown_fields() {
        for (fields, extensions) in [(0x20, vec![]), (0x0f, vec![U256::from(1)])] {
            assert!(
                discovered_domain(IERC2612::eip712DomainReturn {
                    fields: [fields].into(),
                    name: String::new(),
                    version: String::new(),
                    chainId: U256::ZERO,
                    verifyingContract: Address::ZERO,
                    salt: B256::ZERO,
                    extensions,
                })
                .is_err()
            );
        }
    }

    #[test]
    fn domain_validation_rejects_mismatched_separator_chain_and_contract() {
        let chain = U256::from(1);
        let token = Address::repeat_byte(1);
        let domain = Eip712Domain {
            name: Some("Token".into()),
            version: Some("2".into()),
            chain_id: Some(chain),
            verifying_contract: Some(token),
            ..Default::default()
        };
        let separator = domain.separator();
        assert!(validate_domain(&domain, separator, chain, token).is_ok());
        assert!(validate_domain(&domain, B256::ZERO, chain, token).is_err());
        assert!(validate_domain(&domain, separator, U256::from(2), token).is_err());
        assert!(validate_domain(&domain, separator, chain, Address::ZERO).is_err());
    }
}
