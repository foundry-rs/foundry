use crate::evm::{calldata::CallData, op, vm::Vm, VAL_0_B, VAL_1, VAL_1_B, U256};
use alloy_dyn_abi::DynSolType;

// Executes the EVM until the start of a function is reached (vm.calldata selector)
pub fn execute_until_function_start<T, U>(vm: &mut Vm<T, U>, gas_limit: u32) -> Option<u32>
where
    T: Clone + std::fmt::Debug + std::cmp::Eq,
    U: CallData<T>,
{
    let mut gas_used = 0;
    let mut found = false;
    while !vm.stopped {
        let ret = match vm.step() {
            Ok(v) => v,
            Err(_e) => {
                // println!("{}", _e);
                break;
            }
        };
        gas_used += ret.gas_used;
        if gas_used > gas_limit {
            break;
        }

        if found && ret.op == op::JUMPI {
            return Some(gas_used);
        }

        if ret.op == op::EQ || ret.op == op::XOR || ret.op == op::SUB {
            let p = vm
                .stack
                .peek()
                .expect("always safe unless bug in vm.rs")
                .data;
            if (ret.op == op::EQ && p == VAL_1_B) || (ret.op != op::EQ && p == VAL_0_B) {
                if let Some(v) = ret.fa {
                    if v.data[28..32] == vm.calldata.selector() {
                        found = true;
                    }
                }
            }
        }
    }
    None
}

pub fn and_mask_to_type(mask: U256) -> Option<DynSolType> {
    if mask.is_zero() {
        return None;
    }

    if (mask & (mask + VAL_1)).is_zero() {
        // 0x0000ffff
        let bl = mask.bit_len();
        if bl % 8 == 0 {
            return Some(if bl == 160 {
                DynSolType::Address
            } else {
                DynSolType::Uint(bl)
            });
        }
    } else {
        // 0xffff0000
        let mask = U256::from_le_bytes(mask.to_be_bytes() as [u8; 32]);
        if (mask & (mask + VAL_1)).is_zero() {
            let bl = mask.bit_len();
            if bl % 8 == 0 {
                return Some(DynSolType::FixedBytes(bl / 8));
            }
        }
    }
    None
}
