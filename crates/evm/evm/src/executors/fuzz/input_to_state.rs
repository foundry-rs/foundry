//! Finds argument words that feed a call's comparisons by running the call again with one word
//! shifted by one, and solves those comparisons for the word.
//!
//! Only comparisons whose operand moves by exactly one with a static `uint`, `int` or `address`
//! word are solved. Scaled, shifted or hashed operands, and words inside dynamic data, are not.

use super::frontier::comparison_result;
use crate::{executors::Executor, inspectors::CmpOperands};
use alloy_dyn_abi::DynSolType;
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, I256, U256,
    map::{HashMap, HashSet},
};
use evm2::interpreter::op as opcode;
use foundry_evm_core::evm::FoundryEvmNetwork;
use foundry_evm_fuzz::{
    BasicTxDetails,
    sequence::{SELECTOR_LEN, WordPatch},
};

/// Most argument words shifted per call, so wide static arguments do not multiply the cost.
const MAX_SHIFTED_WORDS: usize = 16;

/// Returns patches for the comparisons in `base` whose operand moves with one `uint`, `int` or
/// `address` argument word of `tx` at slope 1 or -1.
///
/// `base` is the comparison log of `tx` run on the executor's current state. Each shifted run is a
/// call that is not committed.
pub(in crate::executors) fn input_to_state_patches<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    tx: &BasicTxDetails,
    function: &Function,
    base: &[CmpOperands],
) -> Vec<WordPatch> {
    if base.is_empty() {
        return Vec::new();
    }
    let calldata = &tx.call_details.calldata;
    let mut patches = Vec::new();
    let words = value_words(function);
    for &ValueWord { offset, max } in words.iter().take(MAX_SHIFTED_WORDS) {
        let Some(bytes) = calldata.get(offset..offset + 32) else { break };
        let word = U256::from_be_slice(bytes);
        let delta = if word == max { U256::MAX } else { U256::ONE };
        let mut shifted_calldata = calldata.to_vec();
        shifted_calldata[offset..offset + 32]
            .copy_from_slice(&word.wrapping_add(delta).to_be_bytes::<32>());
        let Ok(result) = executor.call_raw(
            tx.sender,
            tx.call_details.target,
            shifted_calldata.into(),
            tx.call_details.value.unwrap_or_default(),
        ) else {
            continue;
        };
        let shifted = result.evm_cmp_values.unwrap_or_default();
        patches.extend(solve(base, &shifted, offset, word, delta));
    }
    let mut seen = HashSet::<WordPatch>::default();
    patches.retain(|patch| seen.insert(*patch));
    trace!(
        target: "corpus",
        words = words.len(),
        patches = patches.len(),
        "measured input-to-state patches"
    );
    patches
}

/// A `uint`, `int` or `address` head word of a call's arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ValueWord {
    /// Byte offset of the word in the calldata, counting the selector.
    offset: usize,
    /// The largest value the word can hold.
    max: U256,
}

/// Returns the `uint`, `int` and `address` head words of `function`'s arguments.
fn value_words(function: &Function) -> Vec<ValueWord> {
    let mut words = Vec::new();
    let mut offset = SELECTOR_LEN;
    for input in &function.inputs {
        let Ok(ty) = input.selector_type().parse::<DynSolType>() else { break };
        if !push_value_words(&ty, &mut offset, &mut words) {
            break;
        }
    }
    words
}

/// Adds the value words of `ty` at `offset` and moves `offset` past its head. Returns `false`
/// for a type whose head size is unknown.
fn push_value_words(ty: &DynSolType, offset: &mut usize, words: &mut Vec<ValueWord>) -> bool {
    if !ty.is_dynamic() {
        let at = *offset;
        match ty {
            DynSolType::Uint(bits) => {
                words.push(ValueWord { offset: at, max: U256::MAX >> (256 - bits) })
            }
            DynSolType::Int(bits) => {
                words.push(ValueWord { offset: at, max: U256::MAX >> (257 - bits) })
            }
            DynSolType::Address => words.push(ValueWord { offset: at, max: U256::MAX >> 96 }),
            DynSolType::Bool | DynSolType::FixedBytes(_) | DynSolType::Function => {}
            DynSolType::FixedArray(inner, len) => {
                return (0..*len).all(|_| push_value_words(inner, offset, words));
            }
            DynSolType::Tuple(types) => {
                return types.iter().all(|ty| push_value_words(ty, offset, words));
            }
            _ => return false,
        }
    }
    // A dynamic value's head word holds the offset of its data.
    *offset += 32;
    true
}

/// Solves each comparison of `base` that, in `shifted`, moved by exactly `delta` or `-delta` on
/// one side while the other side stayed the same.
///
/// `shifted` is the log of the same call with the word at `offset` changed from `word` to
/// `word + delta`. Comparisons are matched by site and occurrence, and a site is used only when it
/// occurs the same number of times in both logs.
fn solve(
    base: &[CmpOperands],
    shifted: &[CmpOperands],
    offset: usize,
    word: U256,
    delta: U256,
) -> Vec<WordPatch> {
    let shifted = by_site(shifted);
    let mut patches = Vec::new();
    for (site, base) in by_site(base) {
        let Some(shifted) = shifted.get(&site).filter(|shifted| shifted.len() == base.len()) else {
            continue;
        };
        for (cmp, moved) in base.into_iter().zip(shifted) {
            let Some((is_op1, slope)) = moved_operand(cmp, moved) else { continue };
            let operand = if is_op1 { cmp.op1 } else { cmp.op2 };
            let Some(target) = flip_target(cmp, is_op1) else { continue };
            let step = target.wrapping_sub(operand);
            let word = if slope == delta {
                word.wrapping_add(step)
            } else if slope == delta.wrapping_neg() {
                word.wrapping_sub(step)
            } else {
                continue;
            };
            patches.push(WordPatch { offset, word });
        }
    }
    patches
}

/// Returns whether op1 (else op2) of `moved` differs from `base` and by how much, when exactly one
/// operand differs.
fn moved_operand(base: &CmpOperands, moved: &CmpOperands) -> Option<(bool, U256)> {
    let (d1, d2) = (moved.op1.wrapping_sub(base.op1), moved.op2.wrapping_sub(base.op2));
    match (d1.is_zero(), d2.is_zero()) {
        (false, true) => Some((true, d1)),
        (true, false) => Some((false, d2)),
        _ => None,
    }
}

/// Groups `log` by comparison site, keeping each site's occurrences in execution order.
fn by_site(log: &[CmpOperands]) -> HashMap<(Address, usize, u8), Vec<&CmpOperands>> {
    let mut sites = HashMap::<_, Vec<_>>::default();
    for cmp in log {
        sites.entry((cmp.address, cmp.pc, cmp.opcode)).or_default().push(cmp);
    }
    sites
}

/// Returns whether a false ordering `opcode` turns true when the operand being solved is above the
/// other one, not below it.
const fn needs_higher_operand(opcode: u8, is_op1: bool) -> bool {
    matches!(opcode, opcode::GT | opcode::SGT) == is_op1
}

/// Returns the value for one operand of `cmp`, op1 when `is_op1`, that flips its result while the
/// other operand stays the same.
fn flip_target(cmp: &CmpOperands, is_op1: bool) -> Option<U256> {
    let result = comparison_result(cmp)?;
    let other = if is_op1 { cmp.op2 } else { cmp.op1 };
    match cmp.opcode {
        opcode::EQ => Some(if result { other.wrapping_add(U256::ONE) } else { other }),
        opcode::ISZERO => Some(if result { U256::ONE } else { U256::ZERO }),
        // Equal operands make a true ordering false. A false ordering needs the operand one past
        // the other, on the side the comparison asks for.
        opcode::LT | opcode::GT | opcode::SLT | opcode::SGT if result => Some(other),
        opcode::LT | opcode::GT => {
            if needs_higher_operand(cmp.opcode, is_op1) {
                other.checked_add(U256::ONE)
            } else {
                other.checked_sub(U256::ONE)
            }
        }
        opcode::SLT | opcode::SGT => {
            let other = I256::from_raw(other);
            let target = if needs_higher_operand(cmp.opcode, is_op1) {
                other.checked_add(I256::ONE)
            } else {
                other.checked_sub(I256::ONE)
            };
            target.map(I256::into_raw)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmp(opcode: u8, op1: U256, op2: U256) -> CmpOperands {
        CmpOperands { op1, op2, pc: 7, address: Address::ZERO, opcode }
    }

    fn patch(word: U256) -> WordPatch {
        WordPatch { offset: 4, word }
    }

    #[test]
    fn solves_balance_offset() {
        let before = U256::from(10).pow(U256::from(24));
        let target = U256::MAX >> 2;
        let amount = U256::from(5);
        let base = [cmp(opcode::EQ, before + amount, target)];
        let shifted = [cmp(opcode::EQ, before + amount + U256::ONE, target)];
        assert_eq!(solve(&base, &shifted, 4, amount, U256::ONE), [patch(target - before)]);
    }

    #[test]
    fn solves_negated_operand() {
        let value = U256::MAX >> 3;
        let x = U256::from(100);
        let base = [cmp(opcode::EQ, value - x, U256::from(7))];
        let shifted = [cmp(opcode::EQ, value - x - U256::ONE, U256::from(7))];
        assert_eq!(solve(&base, &shifted, 4, x, U256::ONE), [patch(value - U256::from(7))]);
    }

    #[test]
    fn solves_with_negative_shift() {
        let x = U256::MAX;
        let base = [cmp(opcode::EQ, x, U256::from(9))];
        let shifted = [cmp(opcode::EQ, x - U256::ONE, U256::from(9))];
        assert_eq!(solve(&base, &shifted, 4, x, U256::MAX), [patch(U256::from(9))]);
    }

    #[test]
    fn skips_comparisons_that_do_not_move_by_one_on_one_side() {
        let x = U256::from(5);
        for (base, shifted) in [
            // Both sides moved.
            (cmp(opcode::EQ, x, x + U256::ONE), cmp(opcode::EQ, x + U256::ONE, x + U256::from(2))),
            // Slope 2.
            (
                cmp(opcode::EQ, x * U256::from(2), U256::ONE),
                cmp(opcode::EQ, x * U256::from(2) + U256::from(2), U256::ONE),
            ),
            // Nothing moved.
            (cmp(opcode::EQ, U256::ONE, U256::from(2)), cmp(opcode::EQ, U256::ONE, U256::from(2))),
        ] {
            assert!(solve(&[base], &[shifted], 4, x, U256::ONE).is_empty(), "{base:?}");
        }
    }

    #[test]
    fn skips_sites_whose_occurrence_count_changed() {
        let x = U256::from(3);
        let base = [cmp(opcode::LT, U256::ZERO, x), cmp(opcode::LT, U256::ONE, x)];
        let shifted = [
            cmp(opcode::LT, U256::ZERO, x + U256::ONE),
            cmp(opcode::LT, U256::ONE, x + U256::ONE),
            cmp(opcode::LT, U256::from(2), x + U256::ONE),
        ];
        assert!(solve(&base, &shifted, 4, x, U256::ONE).is_empty());
    }

    #[test]
    fn solves_ordering_through_offset() {
        let before = U256::from(1000);
        let t = U256::from(5000);
        let x = U256::from(10);
        // `before + x > t` is false, so the sum must rise one past `t`.
        let base = [cmp(opcode::GT, before + x, t)];
        let shifted = [cmp(opcode::GT, before + x + U256::ONE, t)];
        assert_eq!(solve(&base, &shifted, 4, x, U256::ONE), [patch(t + U256::ONE - before)]);
        // `t < before + x` is the same ordering with the moving side on the right.
        let base = [cmp(opcode::LT, t, before + x)];
        let shifted = [cmp(opcode::LT, t, before + x + U256::ONE)];
        assert_eq!(solve(&base, &shifted, 4, x, U256::ONE), [patch(t + U256::ONE - before)]);
    }

    #[test]
    fn solves_each_occurrence_of_a_site() {
        let x = U256::from(3);
        let base =
            [cmp(opcode::EQ, x, U256::from(10)), cmp(opcode::EQ, U256::from(20) - x, U256::ONE)];
        // The second occurrence moves against the word.
        let shifted = [
            cmp(opcode::EQ, x + U256::ONE, U256::from(10)),
            cmp(opcode::EQ, U256::from(20) - x - U256::ONE, U256::ONE),
        ];
        assert_eq!(
            solve(&base, &shifted, 4, x, U256::ONE),
            [patch(U256::from(10)), patch(U256::from(19))]
        );
    }

    #[test]
    fn solves_address_word_at_its_maximum() {
        let max = U256::MAX >> 96;
        let base = [cmp(opcode::EQ, max, U256::from(9))];
        let shifted = [cmp(opcode::EQ, max - U256::ONE, U256::from(9))];
        assert_eq!(solve(&base, &shifted, 4, max, U256::MAX), [patch(U256::from(9))]);
    }

    #[test]
    fn flip_targets_turn_each_comparison_over() {
        let t = U256::from(100);
        let minus_one = I256::MINUS_ONE.into_raw();
        for (opcode, op1, op2, is_op1, expected) in [
            (opcode::EQ, U256::from(5), t, true, Some(t)),
            (opcode::EQ, t, t, true, Some(t + U256::ONE)),
            (opcode::ISZERO, U256::from(5), U256::ZERO, true, Some(U256::ZERO)),
            (opcode::ISZERO, U256::ZERO, U256::ZERO, true, Some(U256::ONE)),
            // `op1 < op2` is false: op1 must drop below op2, or op2 must rise above op1.
            (opcode::LT, U256::from(500), t, true, Some(t - U256::ONE)),
            (opcode::LT, t, U256::from(50), false, Some(t + U256::ONE)),
            // `op1 < op2` is true: equal operands make it false.
            (opcode::LT, U256::from(5), t, true, Some(t)),
            (opcode::GT, U256::from(5), t, true, Some(t + U256::ONE)),
            (opcode::GT, t, U256::from(500), false, Some(t - U256::ONE)),
            (opcode::LT, U256::from(5), U256::ZERO, true, None),
            (
                opcode::SLT,
                U256::ONE,
                minus_one,
                true,
                Some(I256::from_raw(minus_one).checked_sub(I256::ONE).unwrap().into_raw()),
            ),
            (opcode::SGT, minus_one, U256::ONE, true, Some(U256::from(2))),
            (opcode::SLT, U256::ONE, I256::MIN.into_raw(), true, None),
        ] {
            let cmp = cmp(opcode, op1, op2);
            assert_eq!(flip_target(&cmp, is_op1), expected, "{cmp:?}");
            if let Some(target) = expected {
                let flipped = if is_op1 {
                    CmpOperands { op1: target, ..cmp }
                } else {
                    CmpOperands { op2: target, ..cmp }
                };
                assert_ne!(comparison_result(&flipped), comparison_result(&cmp), "{cmp:?}");
            }
        }
    }

    #[test]
    fn value_words_follow_the_abi_head() {
        let function =
            Function::parse("f(uint8,bytes,(address,bool),int16[2],bytes32,uint256[])").unwrap();
        assert_eq!(
            value_words(&function),
            [
                ValueWord { offset: 4, max: U256::from(u8::MAX) },
                ValueWord { offset: 68, max: U256::MAX >> 96 },
                ValueWord { offset: 132, max: U256::from(i16::MAX) },
                ValueWord { offset: 164, max: U256::from(i16::MAX) },
            ]
        );
    }

    #[test]
    fn value_words_skip_a_tuple_with_dynamic_member() {
        let function = Function::parse("f((uint8,bytes),uint16)").unwrap();
        // The tuple is dynamic, so only its offset word occupies the head.
        assert_eq!(value_words(&function), [ValueWord { offset: 36, max: U256::from(u16::MAX) }]);
    }
}
