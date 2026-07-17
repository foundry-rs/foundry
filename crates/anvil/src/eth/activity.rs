//! Activity simulation: generates realistic on-chain traffic every block.
//!
//! When enabled (`--activity` or `anvil_setActivity`), a background task injects real signed
//! transactions from the dev accounts: native transfers, calls to a built-in activity contract
//! (events, storage churn, reverts, gas burn) and mock ERC20 traffic, with configurable
//! per-block volume and outcome weights.

use crate::eth::{EthApi, error::Result};
use alloy_primitives::{Address, Bytes, TxKind, U256, address, bytes};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use alloy_sol_types::{SolCall, sol};
use anvil_core::types::{ActivityKind, ActivityOptions};
use foundry_common::tempo::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS,
};
use foundry_primitives::FoundryNetwork;
use futures::StreamExt;
use rand_08::{Rng, SeedableRng, rngs::StdRng};
use std::time::Duration;

/// Address of the built-in activity contract, installed when activity is enabled.
pub const ACTIVITY_ADDRESS: Address = address!("0x00000000000000000000000000000000000ac717");

/// Address of the built-in mock ERC20, installed when activity is enabled.
pub const ACTIVITY_TOKEN_ADDRESS: Address = address!("0x00000000000000000000000000000000000ac720");

// Runtime bytecode of `Activity.sol` / `ActivityToken.sol` (solc 0.8.30, default forge profile).
// Sources live in `contracts/` next to this module; recompile with `forge build --use 0.8.30`.
pub(crate) const ACTIVITY_RUNTIME_CODE: Bytes = bytes!(
    "0x608060405234801561000f575f5ffd5b5060043610610091575f3560e01c80637dacda03116100645780637dacda031461012f5780639c0e3f7a1461014b578063c74e820e14610167578063d408eb6e14610185578063d7d58f5b146101a157610091565b80632abc3d4e146100955780634ad5d16f146100c55780635e383d21146100e157806361bc221a14610111575b5f5ffd5b6100af60048036038101906100aa9190610548565b6101bd565b6040516100bc91906105e3565b60405180910390f35b6100df60048036038101906100da9190610548565b610263565b005b6100fb60048036038101906100f69190610548565b6102ba565b6040516101089190610612565b60405180910390f35b6101196102cf565b6040516101269190610612565b60405180910390f35b6101496004803603810190610144919061068c565b6102d4565b005b610165600480360381019061016091906106d7565b610313565b005b61016f61032d565b60405161017c919061072d565b60405180910390f35b61019f600480360381019061019a919061079b565b610333565b005b6101bb60048036038101906101b69190610548565b610372565b005b600281815481106101cc575f80fd5b905f5260205f20015f9150905080546101e490610813565b80601f016020809104026020016040519081016040528092919081815260200182805461021090610813565b801561025b5780601f106102325761010080835404028352916020019161025b565b820191905f5260205f20905b81548152906001019060200180831161023e57829003601f168201915b505050505081565b5f60035490505f5f90505b828110156102ae578181604051602001610289929190610883565b604051602081830303815290604052805190602001209150808060010191505061026e565b50806003819055505050565b6001602052805f5260405f205f915090505481565b5f5481565b6002828290918060018154018082558091505060019003905f5260205f20015f90919290919290919290919250918261030e929190610a85565b505050565b8060015f8481526020019081526020015f20819055505050565b60035481565b81816040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610369929190610b9c565b60405180910390fd5b5f5f5490505f5f90505b828110156104f7575f600382846103939190610beb565b61039d9190610c4b565b90505f81036103fa573373ffffffffffffffffffffffffffffffffffffffff1682846103c99190610beb565b7fc05b373e05c47417d9c7204807552389e512c0e21cbc01a03d1554561080ac6e60405160405180910390a36104e9565b6001810361047657818361040e9190610beb565b7f44836be0fd3b72cff7564f074fc591a00c481717a3b2875a766197662df53247838561043b9190610beb565b3360405160200161044d929190610cf0565b60405160208183030381529060405260405161046991906105e3565b60405180910390a26104e8565b81836104829190610beb565b3073ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167f6ca16b72f6875e6f006f27693d2789f0058dfe87daeada50ca257e17b411d1b4436040516104df9190610612565b60405180910390a45b5b50808060010191505061037c565b5081816105049190610beb565b5f819055505050565b5f5ffd5b5f5ffd5b5f819050919050565b61052781610515565b8114610531575f5ffd5b50565b5f813590506105428161051e565b92915050565b5f6020828403121561055d5761055c61050d565b5b5f61056a84828501610534565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b8281835e5f83830152505050565b5f601f19601f8301169050919050565b5f6105b582610573565b6105bf818561057d565b93506105cf81856020860161058d565b6105d88161059b565b840191505092915050565b5f6020820190508181035f8301526105fb81846105ab565b905092915050565b61060c81610515565b82525050565b5f6020820190506106255f830184610603565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261064c5761064b61062b565b5b8235905067ffffffffffffffff8111156106695761066861062f565b5b60208301915083600182028301111561068557610684610633565b5b9250929050565b5f5f602083850312156106a2576106a161050d565b5b5f83013567ffffffffffffffff8111156106bf576106be610511565b5b6106cb85828601610637565b92509250509250929050565b5f5f604083850312156106ed576106ec61050d565b5b5f6106fa85828601610534565b925050602061070b85828601610534565b9150509250929050565b5f819050919050565b61072781610715565b82525050565b5f6020820190506107405f83018461071e565b92915050565b5f5f83601f84011261075b5761075a61062b565b5b8235905067ffffffffffffffff8111156107785761077761062f565b5b60208301915083600182028301111561079457610793610633565b5b9250929050565b5f5f602083850312156107b1576107b061050d565b5b5f83013567ffffffffffffffff8111156107ce576107cd610511565b5b6107da85828601610746565b92509250509250929050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52602260045260245ffd5b5f600282049050600182168061082a57607f821691505b60208210810361083d5761083c6107e6565b5b50919050565b5f819050919050565b61085d61085882610715565b610843565b82525050565b5f819050919050565b61087d61087882610515565b610863565b82525050565b5f61088e828561084c565b60208201915061089e828461086c565b6020820191508190509392505050565b5f82905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f819050815f5260205f209050919050565b5f6020601f8301049050919050565b5f82821b905092915050565b5f600883026109417fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82610906565b61094b8683610906565b95508019841693508086168417925050509392505050565b5f819050919050565b5f61098661098161097c84610515565b610963565b610515565b9050919050565b5f819050919050565b61099f8361096c565b6109b36109ab8261098d565b848454610912565b825550505050565b5f5f905090565b6109ca6109bb565b6109d5818484610996565b505050565b5b818110156109f8576109ed5f826109c2565b6001810190506109db565b5050565b601f821115610a3d57610a0e816108e5565b610a17846108f7565b81016020851015610a26578190505b610a3a610a32856108f7565b8301826109da565b50505b505050565b5f82821c905092915050565b5f610a5d5f1984600802610a42565b1980831691505092915050565b5f610a758383610a4e565b9150826002028217905092915050565b610a8f83836108ae565b67ffffffffffffffff811115610aa857610aa76108b8565b5b610ab28254610813565b610abd8282856109fc565b5f601f831160018114610aea575f8415610ad8578287013590505b610ae28582610a6a565b865550610b49565b601f198416610af8866108e5565b5f5b82811015610b1f57848901358255600182019150602085019450602081019050610afa565b86831015610b3c5784890135610b38601f891682610a4e565b8355505b6001600288020188555050505b50505050505050565b5f82825260208201905092915050565b828183375f83830152505050565b5f610b7b8385610b52565b9350610b88838584610b62565b610b918361059b565b840190509392505050565b5f6020820190508181035f830152610bb5818486610b70565b90509392505050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f610bf582610515565b9150610c0083610515565b9250828201905080821115610c1857610c17610bbe565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f610c5582610515565b9150610c6083610515565b925082610c7057610c6f610c1e565b5b828206905092915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610ca482610c7b565b9050919050565b5f8160601b9050919050565b5f610cc182610cab565b9050919050565b5f610cd282610cb7565b9050919050565b610cea610ce582610c9a565b610cc8565b82525050565b5f610cfb828561086c565b602082019150610d0b8284610cd9565b601482019150819050939250505056fea26469706673582212204cf7552c3f1b155e79be963f952ddb51380ce31b818eaf18e8ba904997f7286f64736f6c634300081e0033"
);
pub(crate) const ACTIVITY_TOKEN_RUNTIME_CODE: Bytes = bytes!(
    "0x608060405234801561000f575f5ffd5b506004361061009c575f3560e01c806340c10f191161006457806340c10f191461015a57806370a082311461017657806395d89b41146101a6578063a9059cbb146101c4578063dd62ed3e146101f45761009c565b806306fdde03146100a0578063095ea7b3146100be57806318160ddd146100ee57806323b872dd1461010c578063313ce5671461013c575b5f5ffd5b6100a8610224565b6040516100b591906107c0565b60405180910390f35b6100d860048036038101906100d39190610871565b61025d565b6040516100e591906108c9565b60405180910390f35b6100f661034a565b60405161010391906108f1565b60405180910390f35b6101266004803603810190610121919061090a565b61034f565b60405161013391906108c9565b60405180910390f35b6101446104f4565b6040516101519190610975565b60405180910390f35b610174600480360381019061016f9190610871565b6104f9565b005b610190600480360381019061018b919061098e565b6105cc565b60405161019d91906108f1565b60405180910390f35b6101ae6105e1565b6040516101bb91906107c0565b60405180910390f35b6101de60048036038101906101d99190610871565b61061a565b6040516101eb91906108c9565b60405180910390f35b61020e600480360381019061020991906109b9565b610730565b60405161021b91906108f1565b60405180910390f35b6040518060400160405280600e81526020017f416374697669747920546f6b656e00000000000000000000000000000000000081525081565b5f8160025f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f8573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055508273ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b9258460405161033891906108f1565b60405180910390a36001905092915050565b5f5481565b5f8160025f8673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f8282546103d79190610a24565b925050819055508160015f8673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f82825461042a9190610a24565b925050819055508160015f8573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f82825461047d9190610a57565b925050819055508273ffffffffffffffffffffffffffffffffffffffff168473ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef846040516104e191906108f1565b60405180910390a3600190509392505050565b601281565b805f5f8282546105099190610a57565b925050819055508060015f8473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f82825461055c9190610a57565b925050819055508173ffffffffffffffffffffffffffffffffffffffff165f73ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef836040516105c091906108f1565b60405180910390a35050565b6001602052805f5260405f205f915090505481565b6040518060400160405280600381526020017f414354000000000000000000000000000000000000000000000000000000000081525081565b5f8160015f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f8282546106679190610a24565b925050819055508160015f8573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f8282546106ba9190610a57565b925050819055508273ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef8460405161071e91906108f1565b60405180910390a36001905092915050565b6002602052815f5260405f20602052805f5260405f205f91509150505481565b5f81519050919050565b5f82825260208201905092915050565b8281835e5f83830152505050565b5f601f19601f8301169050919050565b5f61079282610750565b61079c818561075a565b93506107ac81856020860161076a565b6107b581610778565b840191505092915050565b5f6020820190508181035f8301526107d88184610788565b905092915050565b5f5ffd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f61080d826107e4565b9050919050565b61081d81610803565b8114610827575f5ffd5b50565b5f8135905061083881610814565b92915050565b5f819050919050565b6108508161083e565b811461085a575f5ffd5b50565b5f8135905061086b81610847565b92915050565b5f5f60408385031215610887576108866107e0565b5b5f6108948582860161082a565b92505060206108a58582860161085d565b9150509250929050565b5f8115159050919050565b6108c3816108af565b82525050565b5f6020820190506108dc5f8301846108ba565b92915050565b6108eb8161083e565b82525050565b5f6020820190506109045f8301846108e2565b92915050565b5f5f5f60608486031215610921576109206107e0565b5b5f61092e8682870161082a565b935050602061093f8682870161082a565b92505060406109508682870161085d565b9150509250925092565b5f60ff82169050919050565b61096f8161095a565b82525050565b5f6020820190506109885f830184610966565b92915050565b5f602082840312156109a3576109a26107e0565b5b5f6109b08482850161082a565b91505092915050565b5f5f604083850312156109cf576109ce6107e0565b5b5f6109dc8582860161082a565b92505060206109ed8582860161082a565b9150509250929050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f610a2e8261083e565b9150610a398361083e565b9250828203905081811115610a5157610a506109f7565b5b92915050565b5f610a618261083e565b9150610a6c8361083e565b9250828201905080821115610a8457610a836109f7565b5b9291505056fea2646970667358221220050322c53eefa020bf4ecc58ef08f2e26996e36f181d23079147b6ee543e12dd64736f6c634300081e0033"
);

sol! {
    interface IActivity {
        function emitEvents(uint256 n);
        function write(uint256 slot, uint256 value);
        function push(bytes data);
        function revertWith(string reason);
        function burnGas(uint256 rounds);
    }

    interface IActivityToken {
        function mint(address to, uint256 value);
        function transfer(address to, uint256 value) returns (bool);
        function approve(address spender, uint256 value) returns (bool);
    }
}

/// Generates a batch of activity transactions per mined block.
pub struct ActivityGenerator {
    api: EthApi<FoundryNetwork>,
    rng: StdRng,
    accounts: Vec<Address>,
    /// Whether the mock ERC20 has been seeded with balances.
    seeded: bool,
    /// Number of intentionally-pending (gapped-nonce) transactions submitted so far.
    pending_submitted: usize,
}

/// Cap on intentionally-pending transactions, so the pool does not grow unbounded.
const MAX_PENDING: usize = 64;

/// Gas limit for generated contract calls; skips estimation (which fails for reverts).
const CALL_GAS: u64 = 300_000;

/// Gas limit for TIP-20 precompile transfers, covering T1+ transaction accounting.
const TIP20_GAS: u64 = 1_000_000;

/// TIP-20 fee tokens available in anvil's Tempo genesis.
const TIP20_TOKENS: &[Address] =
    &[PATH_USD_ADDRESS, ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, THETA_USD_ADDRESS];

impl ActivityGenerator {
    pub fn new(api: EthApi<FoundryNetwork>) -> Self {
        let seed = api.activity_config().read().as_ref().and_then(|config| config.seed);
        Self {
            api,
            rng: seed.map_or_else(StdRng::from_entropy, StdRng::seed_from_u64),
            accounts: Vec::new(),
            seeded: false,
            pending_submitted: 0,
        }
    }

    /// Drives activity generation: on every new block (or idle tick), injects the next batch.
    pub async fn run(mut self) {
        let mut notifications = self.api.backend.new_block_notifications();
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                block = notifications.next() => if block.is_none() { return },
                _ = tick.tick() => {}
            }
            let Some(config) = self.api.activity_config().read().clone() else { continue };
            // Wait until the previous batch has been mined before topping up.
            if self.api.has_ready_pool_transactions() {
                continue;
            }
            self.inject_batch(&config).await;
        }
    }

    async fn inject_batch(&mut self, config: &ActivityOptions) {
        if self.accounts.is_empty() {
            self.accounts = self.api.accounts().unwrap_or_default();
            if self.accounts.len() < 2 {
                return;
            }
        }
        if !self.seeded {
            self.seed_token_balances().await;
            self.seeded = true;
        }

        let kinds = self.kinds(config);
        if kinds.is_empty() {
            return;
        }
        let count = self.rng.gen_range(config.txs.min..=config.txs.max);
        for _ in 0..count {
            let outcome = self.rng.gen_range(0..100u8);
            let result = if outcome < config.reverted {
                self.send_reverting().await
            } else if outcome < config.reverted.saturating_add(config.pending)
                && self.pending_submitted < MAX_PENDING
            {
                self.pending_submitted += 1;
                self.send_pending().await
            } else {
                let kind = kinds[self.rng.gen_range(0..kinds.len())];
                self.send_kind(kind, config).await
            };
            if let Err(err) = result {
                trace!(target: "node::activity", %err, "failed to inject activity transaction");
            }
        }
    }

    /// Returns the enabled kinds, defaulting per network.
    fn kinds(&self, config: &ActivityOptions) -> Vec<ActivityKind> {
        let is_tempo = self.api.backend.is_tempo();
        match &config.mix {
            Some(mix) if !mix.is_empty() => mix
                .iter()
                .copied()
                .filter(|kind| match kind {
                    ActivityKind::Transfer | ActivityKind::Erc20 => !is_tempo,
                    ActivityKind::Tip20 => is_tempo,
                    ActivityKind::Contract | ActivityKind::State => true,
                })
                .collect(),
            _ if is_tempo => {
                vec![ActivityKind::Tip20, ActivityKind::Contract, ActivityKind::State]
            }
            _ => vec![
                ActivityKind::Transfer,
                ActivityKind::Contract,
                ActivityKind::Erc20,
                ActivityKind::State,
            ],
        }
    }

    async fn send_kind(
        &mut self,
        kind: ActivityKind,
        config: &ActivityOptions,
    ) -> Result<()> {
        match kind {
            ActivityKind::Transfer => {
                let (from, to) = self.pick_pair();
                let value = self.sample_value(config);
                self.send(
                    from,
                    TransactionRequest {
                        to: Some(TxKind::Call(to)),
                        value: Some(value),
                        ..Default::default()
                    },
                )
                .await
            }
            ActivityKind::Contract => {
                let n = self.rng.gen_range(config.logs.min..=config.logs.max);
                let input = IActivity::emitEventsCall { n: U256::from(n) }.abi_encode();
                self.call(ACTIVITY_ADDRESS, input).await
            }
            ActivityKind::State => {
                let input = match self.rng.gen_range(0..3u8) {
                    0 => IActivity::writeCall {
                        slot: U256::from(self.rng.gen_range(0..1024u64)),
                        value: U256::from(self.rng.r#gen::<u64>()),
                    }
                    .abi_encode(),
                    1 => IActivity::pushCall { data: Bytes::from(self.rng.r#gen::<[u8; 32]>()) }
                        .abi_encode(),
                    _ => {
                        IActivity::burnGasCall { rounds: U256::from(self.rng.gen_range(1..64u64)) }
                            .abi_encode()
                    }
                };
                self.call(ACTIVITY_ADDRESS, input).await
            }
            ActivityKind::Erc20 => {
                let to = self.pick_account();
                let value = self.sample_value(config);
                let input = if self.rng.gen_bool(0.8) {
                    IActivityToken::transferCall { to, value }.abi_encode()
                } else {
                    IActivityToken::approveCall { spender: to, value }.abi_encode()
                };
                self.call(ACTIVITY_TOKEN_ADDRESS, input).await
            }
            ActivityKind::Tip20 => {
                let token = TIP20_TOKENS[self.rng.gen_range(0..TIP20_TOKENS.len())];
                let (from, to) = self.pick_pair();
                let value = self.sample_value(config);
                let input = IActivityToken::transferCall { to, value }.abi_encode();
                self.send(
                    from,
                    TransactionRequest {
                        to: Some(TxKind::Call(token)),
                        input: Bytes::from(input).into(),
                        gas: Some(TIP20_GAS),
                        ..Default::default()
                    },
                )
                .await
            }
        }
    }

    async fn send_reverting(&mut self) -> Result<()> {
        let input = IActivity::revertWithCall { reason: "activity: intentional revert".into() }
            .abi_encode();
        self.call(ACTIVITY_ADDRESS, input).await
    }

    /// Sends a contract call with a gapped nonce so it stays pending indefinitely.
    async fn send_pending(&mut self) -> Result<()> {
        let from = self.pick_account();
        let nonce = self.api.transaction_count(from, None).await?;
        let gap = self.rng.gen_range(100..1_000u64);
        let input = IActivity::writeCall {
            slot: U256::from(self.rng.gen_range(0..1024u64)),
            value: U256::from(self.rng.r#gen::<u64>()),
        }
        .abi_encode();
        self.send(
            from,
            TransactionRequest {
                to: Some(TxKind::Call(ACTIVITY_ADDRESS)),
                input: Bytes::from(input).into(),
                gas: Some(CALL_GAS),
                nonce: Some(nonce.saturating_to::<u64>() + gap),
                ..Default::default()
            },
        )
        .await
    }

    async fn call(
        &mut self,
        to: Address,
        input: Vec<u8>,
    ) -> Result<()> {
        let from = self.pick_account();
        self.send(
            from,
            TransactionRequest {
                to: Some(TxKind::Call(to)),
                input: Bytes::from(input).into(),
                gas: Some(CALL_GAS),
                ..Default::default()
            },
        )
        .await
    }

    async fn send(
        &mut self,
        from: Address,
        request: TransactionRequest,
    ) -> Result<()> {
        let request = TransactionRequest { from: Some(from), ..request };
        self.api.send_transaction(WithOtherFields::new(request)).await?;
        Ok(())
    }

    /// Seeds token balances for every dev account: TIP-20 fee tokens on Tempo,
    /// mock ERC20 mints otherwise.
    async fn seed_token_balances(&mut self) {
        let balance = U256::from(1_000_000u64) * U256::from(10u64).pow(U256::from(18));
        if self.api.backend.is_tempo() {
            for account in self.accounts.clone() {
                for token in TIP20_TOKENS {
                    let _ = self.api.anvil_deal_tip20(account, *token, balance).await;
                }
            }
            return;
        }
        for account in self.accounts.clone() {
            let input = IActivityToken::mintCall { to: account, value: balance }.abi_encode();
            let _ = self
                .send(
                    account,
                    TransactionRequest {
                        to: Some(TxKind::Call(ACTIVITY_TOKEN_ADDRESS)),
                        input: Bytes::from(input).into(),
                        gas: Some(CALL_GAS),
                        ..Default::default()
                    },
                )
                .await;
        }
    }

    fn pick_account(&mut self) -> Address {
        self.accounts[self.rng.gen_range(0..self.accounts.len())]
    }

    fn pick_pair(&mut self) -> (Address, Address) {
        let from = self.pick_account();
        let mut to = self.pick_account();
        while to == from {
            to = self.pick_account();
        }
        (from, to)
    }

    fn sample_value(&mut self, config: &ActivityOptions) -> U256 {
        let span = config.value.max.saturating_sub(config.value.min);
        if span.is_zero() {
            return config.value.min;
        }
        config.value.min + span * U256::from(self.rng.r#gen::<u64>()) / U256::from(u64::MAX)
    }
}
