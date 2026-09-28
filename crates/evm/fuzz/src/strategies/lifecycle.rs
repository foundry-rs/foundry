//! ABI-derived transaction sequences for stateful invariant campaigns.

use alloy_json_abi::Function;
use alloy_primitives::{Address, Selector};

const MAX_SCENARIOS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LifecycleStep {
    pub(super) function: usize,
    pub(super) dictionary: bool,
}

/// Resolves declared call sequences or derives common lending and vault lifecycles.
pub(super) fn lifecycle_scenarios(
    functions: &[(Address, Function)],
    declared: &[Vec<(Address, Selector)>],
) -> (Vec<Vec<LifecycleStep>>, usize) {
    let mut scenarios = Vec::new();
    if !declared.is_empty() {
        for calls in declared {
            let sequence = calls
                .iter()
                .filter_map(|(target, selector)| {
                    functions.iter().position(|(address, function)| {
                        address == target && function.selector() == *selector
                    })
                })
                .collect::<Vec<_>>();
            if sequence.len() == calls.len() && append_scenario(&mut scenarios, &sequence) {
                break;
            }
        }
    }
    let eager = scenarios.len();

    let names = functions
        .iter()
        .map(|(_, function)| function.name.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut contracts = Vec::<(Address, Vec<usize>)>::new();
    for (function, (address, _)) in functions.iter().enumerate() {
        if let Some((_, indices)) = contracts.iter_mut().find(|(target, _)| target == address) {
            indices.push(function);
        } else {
            contracts.push((*address, vec![function]));
        }
    }

    for (_, actions) in contracts {
        let find = |needles: &[&str]| {
            actions
                .iter()
                .copied()
                .find(|&index| needles.iter().any(|needle| names[index].contains(needle)))
        };
        let Some(actor) = find(&["switchactor", "switch_actor", "selectactor"]) else {
            continue;
        };
        let mut prefix = vec![actor];
        for scope in [
            find(&["switch_spoke", "switchspoke"]),
            find(&["switch_asset", "switchasset"]),
            find(&["switch_vault", "switchvault"]),
        ]
        .into_iter()
        .flatten()
        {
            prefix.push(scope);
        }

        if let (Some(supply), Some(collateral), Some(borrow)) = (
            find(&["supply", "deposit"]),
            find(&["collateral"]),
            find(&["borrow", "drawdebt", "mintdebt"]),
        ) {
            let oracle = find(&["setprice", "set_price", "oracle"]);
            let account_config = find(&["updatespokeconfig", "update_spoke_config"]);
            let liquidation_config = find(&["updateliquidation", "update_liquidation"]);
            let mut collateral_first = prefix.clone();
            collateral_first.extend([collateral, supply, borrow]);
            let mut supply_first = prefix.clone();
            supply_first.extend([supply, collateral, supply, borrow]);
            let mut bases = vec![collateral_first, supply_first];
            if let Some(oracle) = oracle {
                let mut price_transition = prefix.clone();
                price_transition.extend([collateral, supply, oracle, supply, borrow]);
                bases.push(price_transition);
            }

            for terminal in actions.iter().copied().filter(|&index| {
                let name = &names[index];
                name.contains("repay")
                    || name.contains("withdraw")
                    || name.contains("redeem")
                    || name.contains("mintfeeshares")
                    || name.contains("mint_fee_shares")
                    || name.contains("liquidat")
                    || name.starts_with("invariant_")
            }) {
                let terminal_name = &names[terminal];
                for base in &bases {
                    let mut sequence = base.clone();
                    if (terminal_name.contains("repay")
                        || terminal_name.contains("withdraw")
                        || terminal_name.contains("redeem"))
                        && let Some(config) = account_config
                    {
                        sequence.push(config);
                    }
                    if terminal_name.contains("liquidat") {
                        if let Some(oracle) = oracle {
                            sequence.push(oracle);
                        }
                        sequence.push(actor);
                        if let Some(config) = liquidation_config {
                            sequence.push(config);
                        }
                    }
                    if terminal_name.starts_with("invariant_")
                        && let Some(oracle) = oracle
                    {
                        sequence.push(oracle);
                    }
                    sequence.push(terminal);

                    if append_scenario(&mut scenarios, &sequence) {
                        return (scenarios, eager);
                    }
                }
            }
        }

        if let Some(deposit) = actions.iter().copied().find(|&index| {
            let name = &names[index];
            (name.contains("deposit") || name.contains("_mint_"))
                && !name.contains("preview")
                && !name.starts_with("doomsday_")
                && !name.contains("asset_mint")
        }) {
            let mut base = prefix;
            if let Some(mint) = find(&["asset_mint", "faucet"]) {
                base.push(mint);
            }
            if let Some(approve) = find(&["asset_approve", "approve_asset"]) {
                base.push(approve);
            }
            base.push(deposit);

            for terminal in actions.iter().copied().filter(|&index| {
                let name = &names[index];
                (name.contains("withdraw") || name.contains("redeem"))
                    && !name.contains("preview")
                    && !name.starts_with("doomsday_")
            }) {
                let mut sequence = base.clone();
                sequence.push(terminal);
                if append_scenario(&mut scenarios, &sequence) {
                    return (scenarios, eager);
                }
            }
        }
    }

    (scenarios, eager)
}

fn append_scenario(scenarios: &mut Vec<Vec<LifecycleStep>>, sequence: &[usize]) -> bool {
    for dictionary in [true, false] {
        scenarios.push(
            sequence.iter().map(|&function| LifecycleStep { function, dictionary }).collect(),
        );
        if scenarios.len() == MAX_SCENARIOS {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_complete_lending_lifecycles() {
        let target = Address::with_last_byte(1);
        let functions = [
            "switchActor(uint256)",
            "switch_asset(uint256)",
            "supply(uint256,uint256)",
            "setUsingAsCollateral(uint256,bool)",
            "borrow(uint256,uint256)",
            "setPrice(uint256,uint256)",
            "liquidationCall(uint256,uint256)",
        ]
        .into_iter()
        .map(|signature| (target, Function::parse(signature).unwrap()))
        .collect::<Vec<_>>();

        let (scenarios, _) = lifecycle_scenarios(&functions, &[]);
        assert_eq!(
            scenarios[0],
            [0, 1, 3, 2, 4, 5, 0, 6]
                .into_iter()
                .map(|function| LifecycleStep { function, dictionary: true })
                .collect::<Vec<_>>()
        );
        assert!(scenarios.iter().all(|scenario| scenario.len() > 1));
    }

    #[test]
    fn derives_funded_vault_lifecycles() {
        let target = Address::with_last_byte(1);
        let functions = [
            "switchActor(uint256)",
            "switch_asset(uint256)",
            "switch_vault(uint256)",
            "asset_mint(address,uint128)",
            "asset_approve(address,uint128)",
            "superVault_deposit_ASSERTION(uint256)",
            "superVault_requestRedeem(uint256)",
        ]
        .into_iter()
        .map(|signature| (target, Function::parse(signature).unwrap()))
        .collect::<Vec<_>>();

        let (scenarios, _) = lifecycle_scenarios(&functions, &[]);
        assert_eq!(
            scenarios[0],
            [0, 1, 2, 3, 4, 5, 6]
                .into_iter()
                .map(|function| LifecycleStep { function, dictionary: true })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn resolves_declared_cross_contract_sequence() {
        let first = Address::with_last_byte(1);
        let second = Address::with_last_byte(2);
        let functions = [
            (first, Function::parse("prepare(bytes32)").unwrap()),
            (second, Function::parse("exercise(uint256)").unwrap()),
        ];
        let declared =
            vec![vec![(first, functions[0].1.selector()), (second, functions[1].1.selector())]];

        let (scenarios, _) = lifecycle_scenarios(&functions, &declared);

        assert_eq!(
            scenarios[0],
            [0, 1]
                .into_iter()
                .map(|function| LifecycleStep { function, dictionary: true })
                .collect::<Vec<_>>()
        );
    }
}
