use crate::eth::error::BlockchainError;
use ethers::types::U256;

#[derive(Debug)]
pub struct FeeDetails {
    pub gas_price: Option<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
}

impl FeeDetails {
    pub fn new(
        request_gas_price: Option<U256>,
        request_max_fee: Option<U256>,
        request_priority: Option<U256>,
    ) -> Result<FeeDetails, BlockchainError> {
        match (request_gas_price, request_max_fee, request_priority) {
            (gas_price, None, None) => {
                // Legacy request, all default to gas price.
                Ok(FeeDetails {
                    gas_price,
                    max_fee_per_gas: gas_price,
                    max_priority_fee_per_gas: gas_price,
                })
            }
            (_, max_fee, max_priority) => {
                // eip-1559
                // Ensure `max_priority_fee_per_gas` is less or equal to `max_fee_per_gas`.
                if let Some(max_priority) = max_priority {
                    let max_fee = max_fee.unwrap_or_default();
                    if max_priority > max_fee {
                        return Err(BlockchainError::InvalidFeeInput)
                    }
                }
                Ok(FeeDetails {
                    gas_price: max_fee,
                    max_fee_per_gas: max_fee,
                    max_priority_fee_per_gas: max_priority,
                })
            }
        }
    }
}
