//! Callback builders for Earn's scoped SingleZoneEarnRouter.
//!
//! ABI mirrored from tempoxyz/earn at 0c0f141f.
//!
//! Source: <https://github.com/tempoxyz/earn/blob/0c0f141f76f1f3c1f56f414a413f21a9e677d93d/src/router/SingleZoneEarnRouter.sol>

use super::{
    abi::{DepositPayload, IZonePortal},
    encryption,
    l1::L1Args,
};
use alloy_primitives::{Address, B256, Bytes, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use alloy_sol_types::{SolValue, sol};
use clap::Parser;
use eyre::{Result, ensure};
use foundry_cli::json::print_scalar;
use foundry_common::sh_status;

sol! {
    struct ZoneReturn {
        uint256 keyIndex;
        DepositPayload encrypted;
        address refundRecipient;
    }

    struct CallbackData {
        uint8 flow;
        uint128 minVaultAssets;
        uint128 minEarnShares;
        uint128 minOutputAmount;
        bytes32 actionId;
        ZoneReturn zoneReturn;
    }

    #[sol(rpc)]
    interface IEarnRouter {
        function allowedZoneId() external view returns (uint32);
    }
}

/// Build callback bytes to pass to `zone withdraw --callback-data` with nonzero callback gas.
#[derive(Debug, Parser)]
pub(super) struct EarnArgs {
    #[command(subcommand)]
    command: EarnSubcommand,
}

#[derive(Debug, Parser)]
enum EarnSubcommand {
    /// Encode an Earn deposit. Withdraw the router's private asset to --router.
    EncodeDeposit {
        /// Minimum vault assets after conversion, in smallest units.
        #[arg(long)]
        min_vault_assets: u128,
        /// Minimum Earn shares minted, in smallest units.
        #[arg(long)]
        min_earn_shares: u128,
        #[command(flatten)]
        args: BuilderArgs,
    },
    /// Encode an Earn redemption. Withdraw Earn shares to --router.
    EncodeRedeem {
        /// Minimum assets redeemed from the vault, in smallest units.
        #[arg(long)]
        min_vault_assets: u128,
        /// Minimum private assets returned to the zone, in smallest units.
        #[arg(long)]
        min_output_amount: u128,
        #[command(flatten)]
        args: BuilderArgs,
    },
}

#[derive(Debug, Parser)]
struct BuilderArgs {
    /// Scoped SingleZoneEarnRouter address on L1. Also use it as withdrawal --to.
    #[arg(long)]
    router: Address,
    /// Recipient of the encrypted return deposit inside the zone.
    #[arg(long)]
    recipient: Address,
    /// Public L1 refund address if the return deposit fails. Defaults to --recipient.
    #[arg(long)]
    refund_recipient: Option<Address>,
    /// Memo encrypted with the return recipient.
    #[arg(long, default_value_t = B256::ZERO)]
    memo: B256,
    /// Operation identifier. Defaults to the hash of the randomized return ciphertext.
    #[arg(long)]
    action_id: Option<B256>,
    /// Zone ID for the return deposit.
    #[arg(long, env = "ZONE_ID")]
    zone_id: u32,
    /// Zone chain ID used to select the parent RPC.
    #[arg(long, env = "ZONE_CHAIN_ID")]
    zone_chain_id: u64,
    #[command(flatten)]
    l1: L1Args,
}

impl EarnArgs {
    pub(super) async fn run(self) -> Result<()> {
        let (args, flow, min_vault_assets, min_earn_shares, min_output_amount) = match self.command
        {
            EarnSubcommand::EncodeDeposit { args, min_vault_assets, min_earn_shares } => {
                (args, 0, min_vault_assets, min_earn_shares, 0)
            }
            EarnSubcommand::EncodeRedeem { args, min_vault_assets, min_output_amount } => {
                (args, 1, min_vault_assets, 0, min_output_amount)
            }
        };
        ensure!(
            !args.router.is_zero() && !args.recipient.is_zero(),
            "router and recipient must be nonzero"
        );
        let refund_recipient = args.refund_recipient.unwrap_or(args.recipient);
        ensure!(!refund_recipient.is_zero(), "refund recipient must be nonzero");
        let provider = args.l1.provider(args.zone_chain_id, args.zone_id).await?;
        ensure!(
            IEarnRouter::new(args.router, &provider).allowedZoneId().call().await? == args.zone_id,
            "Earn router belongs to a different zone"
        );
        let portal_address = args.l1.portal()?;
        let portal = IZonePortal::new(portal_address, &provider);
        let block = provider.get_block_number().await?;
        let key = portal.encryptionKeyAtBlock(block).block(BlockId::number(block)).call().await?;
        // The router submits the return deposit, so ECIES must bind to its address.
        let encrypted = encryption::encrypt_deposit(
            key.x,
            key.yParity,
            args.recipient,
            args.memo,
            args.router,
            portal_address,
            key.keyIndex,
        )?;
        let action_id = args.action_id.unwrap_or_else(|| keccak256(&encrypted.ciphertext));
        let data = CallbackData {
            flow,
            minVaultAssets: min_vault_assets,
            minEarnShares: min_earn_shares,
            minOutputAmount: min_output_amount,
            actionId: action_id,
            zoneReturn: ZoneReturn {
                keyIndex: key.keyIndex,
                encrypted,
                refundRecipient: refund_recipient,
            },
        };
        sh_status!(
            "Earn action ID: {action_id}. Use withdrawal --to {} with nonzero --callback-gas-limit.",
            args.router
        )?;
        // abi.encode(struct), including its dynamic tuple offset, without a function selector.
        print_scalar(Bytes::from(data.abi_encode()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;
    use alloy_sol_types::SolType;

    #[test]
    fn callback_matches_solidity_struct_encoding() {
        let encrypted = DepositPayload {
            ephemeralPubkeyX: B256::repeat_byte(1),
            ephemeralPubkeyYParity: 1,
            ciphertext: Bytes::from(vec![2; 64]),
            nonce: [3; 12].into(),
            tag: [4; 16].into(),
        };
        let recipient = Address::repeat_byte(5);
        let action = B256::repeat_byte(6);
        type Tuple = sol!((
            uint8,
            uint128,
            uint128,
            uint128,
            bytes32,
            (uint256, (bytes32, uint8, bytes, bytes12, bytes16), address)
        ));
        let expected = Tuple::abi_encode(&(
            1u8,
            7u128,
            0u128,
            9u128,
            action,
            (
                U256::from(10),
                (
                    encrypted.ephemeralPubkeyX,
                    encrypted.ephemeralPubkeyYParity,
                    encrypted.ciphertext.clone(),
                    encrypted.nonce,
                    encrypted.tag,
                ),
                recipient,
            ),
        ));
        let encoded = CallbackData {
            flow: 1,
            minVaultAssets: 7,
            minEarnShares: 0,
            minOutputAmount: 9,
            actionId: action,
            zoneReturn: ZoneReturn {
                keyIndex: U256::from(10),
                encrypted,
                refundRecipient: recipient,
            },
        }
        .abi_encode();
        assert_eq!(encoded, expected);
        assert_eq!(U256::from_be_slice(&encoded[..32]), U256::from(32));
        assert_eq!(<CallbackData as SolValue>::abi_decode(&encoded).unwrap().flow, 1);
    }
}
