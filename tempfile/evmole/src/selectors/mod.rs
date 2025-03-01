use crate::evm::{
    calldata::CallData,
    element::Element,
    op,
    vm::{StepResult, Vm},
    U256, VAL_0_B, VAL_1_B,
};
use crate::Selector;
use alloy_primitives::{uint, hex};
use std::collections::BTreeMap;

mod calldata;
use calldata::CallDataImpl;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Label {
    CallData,
    Signature,
    MulSig,
    SelCmp(Selector, bool),
}

const VAL_FFFFFFFF_B: [u8; 32] = uint!(0xffffffff_U256).to_be_bytes();

fn analyze(
    vm: &mut Vm<Label, CallDataImpl>,
    selectors: &mut BTreeMap<Selector, usize>,
    ret: StepResult<Label>,
    gas_used: &mut u32,
    gas_limit: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    match ret {
          StepResult{op: op::XOR|op::EQ|op::SUB, fa: Some(Element{label: Some(Label::Signature), ..}), sa: Some(s1), ..}
        | StepResult{op: op::XOR|op::EQ|op::SUB, sa: Some(Element{label: Some(Label::Signature), ..}), fa: Some(s1), ..} =>
        {
            let selector: Selector = s1.data[28..32].try_into().expect("4 bytes slice is always convertable to Selector");
            *vm.stack.peek_mut()? = Element{
                data : if ret.op == op::EQ { VAL_0_B } else { VAL_1_B },
                label : Some(Label::SelCmp(selector, false)),
            }
        }

        StepResult{op: op::JUMPI, fa: Some(fa), sa: Some(Element{label: Some(Label::SelCmp(selector, reversed)), ..}), ..} =>
        {
            let pc = if reversed {
                vm.pc + 1
            } else {
                usize::try_from(fa).expect("set to usize in vm.rs")
            };
            selectors.insert(selector, pc);
        }

          StepResult{op: op::LT|op::GT, fa: Some(Element{label: Some(Label::Signature), ..}), ..}
        | StepResult{op: op::LT|op::GT, sa: Some(Element{label: Some(Label::Signature), ..}), ..} =>
        {
            *gas_used += process(vm.clone(), selectors, (gas_limit - *gas_used) / 2);
            let v = vm.stack.peek_mut()?;
            v.data = if v.data == VAL_0_B { VAL_1_B } else { VAL_0_B };
        }

          StepResult{op: op::MUL, fa: Some(Element{label: Some(Label::Signature), ..}), ..}
        | StepResult{op: op::MUL, sa: Some(Element{label: Some(Label::Signature), ..}), ..}
        | StepResult{op: op::SHR, sa: Some(Element{label: Some(Label::MulSig), ..}), ..} =>
        {
            vm.stack.peek_mut()?.label = Some(Label::MulSig);
        }

        // Vyper _selector_section_dense()/_selector_section_sparse()
        // (sig MOD n_buckets) or (sig AND (n_buckets-1))
          StepResult{op: op @ op::MOD, fa: Some(Element{label: Some(Label::MulSig | Label::Signature), ..}), sa: Some(ot), ..}
        | StepResult{op: op @ op::AND, fa: Some(Element{label: Some(Label::Signature), ..}), sa: Some(ot), ..}
        | StepResult{op: op @ op::AND, sa: Some(Element{label: Some(Label::Signature), ..}), fa: Some(ot), ..} =>
        {
            if op == op::AND && ot.data == VAL_FFFFFFFF_B {
                vm.stack.peek_mut()?.label = Some(Label::Signature);
            } else if let Ok(ma) = u8::try_from(ot) {
                let to = if op == op::MOD { ma } else { ma + 1 };
                for m in 1..to {
                    let mut vm_clone = vm.clone();
                    vm_clone.stack.peek_mut()?.data = U256::from(m).to_be_bytes();
                    *gas_used += process(vm_clone, selectors, (gas_limit - *gas_used) / (ma as u32));
                    if *gas_used > gas_limit {
                        break;
                    }
                }
                vm.stack.peek_mut()?.data = VAL_0_B;
            }
        }

          StepResult{op: op::SHR, sa: Some(Element{label: Some(Label::CallData), ..}), ..}
        | StepResult{op: op::DIV, fa: Some(Element{label: Some(Label::CallData), ..}), ..} =>
        {
            let v = vm.stack.peek_mut()?;
            if v.data[28..32] == vm.calldata.selector() {
                v.label = Some(Label::Signature);
            }
        }

          StepResult{op: op::AND, fa: Some(Element{label: Some(Label::CallData), ..}), ..}
        | StepResult{op: op::AND, sa: Some(Element{label: Some(Label::CallData), ..}), ..} =>
        {
            let v = vm.stack.peek_mut()?;
            v.label = Some(Label::CallData);
        }

        StepResult{op: op::ISZERO, fa: Some(Element{label: Some(Label::SelCmp(sel, reversed)), ..}), ..} =>
        {
            let v = vm.stack.peek_mut()?;
            v.label = Some(Label::SelCmp(sel, !reversed));
        }

        StepResult{op: op::ISZERO, fa: Some(Element{label: Some(Label::Signature), ..}), ..} =>
        {
            let v = vm.stack.peek_mut()?;
            v.label = Some(Label::SelCmp([0; 4], false));
        }

        StepResult{op: op::MLOAD, ul: Some(used), ..} =>
        {
            let v = vm.stack.peek_mut()?;
            if used.contains(&Label::CallData) && v.data[28..32] == vm.calldata.selector() {
                v.label = Some(Label::Signature);
            }
        }

        _ => {}
    }
    Ok(())
}

fn process(
    mut vm: Vm<Label, CallDataImpl>,
    selectors: &mut BTreeMap<Selector, usize>,
    gas_limit: u32,
) -> u32 {
    let mut gas_used = 0;
    while !vm.stopped {
        if cfg!(feature = "trace_selectors") {
            println!(
                "selectors: {:?}",
                selectors
                    .iter()
                    .map(|(s, p)| (hex::encode(s), *p))
                    .collect::<Vec<(String, usize)>>()
            );
            println!("{:?}\n", vm);
        }
        let ret = match vm.step() {
            Ok(v) => v,
            Err(_e) => {
                // eprintln!("{}", _e);
                break;
            }
        };
        gas_used += ret.gas_used;
        if gas_used > gas_limit {
            break;
        }

        if analyze(&mut vm, selectors, ret, &mut gas_used, gas_limit).is_err() {
            break;
        }
    }
    gas_used
}

/// Extracts function selectors
///
/// # Arguments
///
/// * `code` - A slice of deployed contract bytecode
/// * `gas_limit` - Maximum allowed gas usage; set to `0` to use defaults
///
/// # Examples
///
/// ```
/// use evmole::function_selectors;
/// use alloy_primitives::hex;
///
/// let code = hex::decode("6080604052348015600e575f80fd5b50600436106030575f3560e01c80632125b65b146034578063b69ef8a8146044575b5f80fd5b6044603f3660046046565b505050565b005b5f805f606084860312156057575f80fd5b833563ffffffff811681146069575f80fd5b925060208401356001600160a01b03811681146083575f80fd5b915060408401356001600160e01b0381168114609d575f80fd5b80915050925092509256").unwrap();
///
/// let selectors: Vec<_> = function_selectors(&code, 0);
///
/// assert_eq!(selectors, vec![[0x21, 0x25, 0xb6, 0x5b], [0xb6, 0x9e, 0xf8, 0xa8]])
/// ```
#[deprecated(since = "0.6.0", note = "Use contract_info(ContractInfoArgs(code).with_selectors()) instead")]
pub fn function_selectors(code: &[u8], gas_limit: u32) -> Vec<Selector> {
    let selectors_with_pc = function_selectors_with_pc(code, gas_limit);
    selectors_with_pc.into_keys().collect()
}

// not public for users yet
pub fn function_selectors_with_pc(code: &[u8], gas_limit: u32) -> BTreeMap<Selector, usize> {
    let vm = Vm::new(code, &CallDataImpl {});
    let mut selectors = BTreeMap::new();
    process(
        vm,
        &mut selectors,
        if gas_limit == 0 {
            5e5 as u32
        } else {
            gas_limit
        },
    );
    selectors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_code() {
        let s = function_selectors(&[], 0);
        assert_eq!(s.len(), 0);
    }
}
