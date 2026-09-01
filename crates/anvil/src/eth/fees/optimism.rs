//! Optimism-specific dynamic EIP-1559 fee rules.

use alloy_consensus::BlockHeader;
use alloy_eips::{calc_next_block_base_fee, eip1559::BaseFeeParams};
use alloy_primitives::Bytes;
use op_alloy_consensus::{decode_holocene_extra_data, decode_jovian_extra_data};

/// Header-derived EIP-1559 rules introduced by the Optimism Holocene and Jovian upgrades.
#[derive(Clone, Copy, Debug)]
pub(super) enum OptimismBaseFeeRules {
    Holocene { extra_data: [u8; 9], params: BaseFeeParams },
    Jovian { extra_data: [u8; 17], params: BaseFeeParams, min_base_fee: u64 },
}

impl OptimismBaseFeeRules {
    pub(super) fn decode(extra_data: &[u8]) -> Option<Self> {
        if let Ok((elasticity, denominator, min_base_fee)) = decode_jovian_extra_data(extra_data) {
            let mut encoded = [0; 17];
            encoded.copy_from_slice(extra_data);
            return Some(Self::Jovian {
                extra_data: encoded,
                params: BaseFeeParams::new(denominator as u128, elasticity as u128),
                min_base_fee,
            });
        }
        if let Ok((elasticity, denominator)) = decode_holocene_extra_data(extra_data) {
            let mut encoded = [0; 9];
            encoded.copy_from_slice(extra_data);
            return Some(Self::Holocene {
                extra_data: encoded,
                params: BaseFeeParams::new(denominator as u128, elasticity as u128),
            });
        }
        None
    }

    pub(super) const fn params(self) -> BaseFeeParams {
        match self {
            Self::Holocene { params, .. } | Self::Jovian { params, .. } => params,
        }
    }

    pub(super) fn extra_data(self) -> Bytes {
        match self {
            Self::Holocene { extra_data, .. } => Bytes::copy_from_slice(&extra_data),
            Self::Jovian { extra_data, .. } => Bytes::copy_from_slice(&extra_data),
        }
    }

    pub(super) const fn is_jovian(self) -> bool {
        matches!(self, Self::Jovian { .. })
    }

    pub(super) fn next_block_base_fee<H: BlockHeader>(self, header: &H) -> u64 {
        let gas_used = match self {
            Self::Holocene { .. } => header.gas_used(),
            Self::Jovian { .. } => {
                header.gas_used().max(header.blob_gas_used().unwrap_or_default())
            }
        };
        let next_base_fee = calc_next_block_base_fee(
            gas_used,
            header.gas_limit(),
            header.base_fee_per_gas().unwrap_or_default(),
            self.params(),
        );
        match self {
            Self::Jovian { min_base_fee, .. } => next_base_fee.max(min_base_fee),
            Self::Holocene { .. } => next_base_fee,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_consensus::Header;

    use super::*;

    #[test]
    fn header_base_fee_rules_match_upstream_blocks() {
        struct Case {
            extra_data: &'static [u8],
            gas_limit: u64,
            gas_used: u64,
            blob_gas_used: u64,
            base_fee: u64,
            expected: u64,
        }

        // Captured parent/child pairs from Base and OP Mainnet across Holocene and Jovian.
        let cases = [
            Case {
                extra_data: &[1, 0, 0, 0, 100, 0, 0, 0, 5, 0, 0, 0, 0, 0, 76, 75, 64],
                gas_limit: 0x17d7_8400,
                gas_used: 0x132_0096,
                blob_gas_used: 0x37_5b00,
                base_fee: 0x4c_4b40,
                expected: 0x4c_4b40,
            },
            Case {
                extra_data: &[1, 0, 0, 0, 250, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0],
                gas_limit: 0x262_5a00,
                gas_used: 0xa5_646d,
                blob_gas_used: 0x22_cd60,
                base_fee: 0x196,
                expected: 0x196,
            },
            Case {
                extra_data: &[0, 0, 0, 0, 50, 0, 0, 0, 2],
                gas_limit: 0x858_3b00,
                gas_used: 0x311_2ab9,
                blob_gas_used: 0,
                base_fee: 0xc_bdf6,
                expected: 0xc_acae,
            },
            Case {
                extra_data: &[0, 0, 0, 0, 250, 0, 0, 0, 2],
                gas_limit: 0x262_5a00,
                gas_used: 0xe6_2170,
                blob_gas_used: 0,
                base_fee: 0xd87,
                expected: 0xd84,
            },
        ];

        for case in cases {
            let rules = OptimismBaseFeeRules::decode(case.extra_data).unwrap();
            let header = Header {
                gas_limit: case.gas_limit,
                gas_used: case.gas_used,
                blob_gas_used: Some(case.blob_gas_used),
                base_fee_per_gas: Some(case.base_fee),
                ..Default::default()
            };

            assert_eq!(rules.next_block_base_fee(&header), case.expected);
            assert_eq!(rules.extra_data().as_ref(), case.extra_data);
        }
    }

    #[test]
    fn jovian_base_fee_uses_blob_gas_when_greater() {
        let extra_data = [1, 0, 0, 0, 250, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0];
        let rules = OptimismBaseFeeRules::decode(&extra_data).unwrap();
        let header = Header {
            gas_limit: 10_000_000_000,
            gas_used: 1_000_000_000,
            blob_gas_used: Some(5_000_000_000),
            base_fee_per_gas: Some(super::super::INITIAL_BASE_FEE),
            ..Default::default()
        };

        let expected = calc_next_block_base_fee(
            header.blob_gas_used.unwrap(),
            header.gas_limit,
            header.base_fee_per_gas.unwrap(),
            BaseFeeParams::new(250, 2),
        );
        assert_eq!(rules.next_block_base_fee(&header), expected);
    }

    #[test]
    fn supplied_header_overrides_cached_optimism_rules() {
        let holocene = [0, 0, 0, 0, 50, 0, 0, 0, 2];
        let jovian = [1, 0, 0, 0, 250, 0, 0, 0, 2, 0, 0, 0, 0, 0, 76, 75, 64];
        let fallback = BaseFeeParams::ethereum();

        for (cached, supplied) in
            [(jovian.as_slice(), holocene.as_slice()), (holocene.as_slice(), jovian.as_slice())]
        {
            let supplied_rules = OptimismBaseFeeRules::decode(supplied).unwrap();
            let header = Header {
                gas_limit: 10_000_000_000,
                gas_used: 1_000_000_000,
                blob_gas_used: Some(5_000_000_000),
                base_fee_per_gas: Some(super::super::INITIAL_BASE_FEE),
                extra_data: Bytes::copy_from_slice(supplied),
                ..Default::default()
            };
            let rules = super::super::BaseFeeRules::Optimism {
                inherited: Some(OptimismBaseFeeRules::decode(cached).unwrap()),
                fallback,
            };

            assert_eq!(
                rules.next_block_base_fee(&header),
                supplied_rules.next_block_base_fee(&header)
            );
        }
    }
}
