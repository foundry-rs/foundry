//! Test helpers for Forge CLI tests.

// This function generates a string containing the code of a Solidity contract
// with a variable init code size.
pub fn generate_large_contract(num_elements: usize) -> String {
    let mut contract_code = String::new();

    contract_code.push_str(
        "// Auto-generated Solidity contract to inflate initcode size\ncontract HugeContract {\n    uint256 public number;\n"
    );

    contract_code.push_str("    uint256[] public largeArray;\n\n    constructor() {\n");
    contract_code.push_str("        largeArray = [");

    for i in 0..num_elements {
        if i != 0 {
            contract_code.push_str(", ");
        }
        contract_code.push_str(&i.to_string());
    }

    contract_code.push_str("];\n");
    contract_code.push_str("    }\n}");

    contract_code
}
