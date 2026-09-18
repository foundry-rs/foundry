// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

interface Vm {
    function envUint(string calldata name) external view returns (uint256);
}

/// Synthetic arithmetic mutants with fixed, independent reference assertions.
/// These fixtures exercise input selection, not deployed contracts.
contract ArithmeticTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 private strategy;
    uint256 private caseId;
    uint256 private pivot;
    bool private mutant;

    function setUp() public {
        strategy = vm.envUint("JEV_STRATEGY");
        caseId = vm.envUint("JEV_CASE");
        pivot = vm.envUint("JEV_PIVOT");
        mutant = vm.envUint("JEV_MUTANT") == 1;
        require(strategy <= 4 && caseId <= 2, "invalid fixture selection");
        require(pivot > 8 && pivot < type(uint32).max - 8, "invalid pivot");
    }

    function testFuzz_arithmetic(uint32 raw) public view {
        uint256 x = input(raw);
        if (caseId == 0) {
            uint256 expected = x < pivot ? x : pivot;
            uint256 actual = mutant && x == pivot - 1 ? pivot : expected;
            require(actual == expected, "synthetic clamp mismatch");
        } else if (caseId == 1) {
            uint256 expected = (x + pivot - 1) / pivot;
            uint256 actual = mutant && x == pivot + 1 ? 1 : expected;
            require(actual == expected, "synthetic page-count mismatch");
        } else {
            uint256 expected = (x + 1) % pivot;
            uint256 actual = mutant && x == pivot - 1 ? pivot : expected;
            require(actual == expected, "synthetic cursor mismatch");
        }
    }

    function input(uint32 raw) private view returns (uint256) {
        // Retain broad exploration for one quarter of inputs in every guided mode.
        if (strategy == 0 || raw % 4 == 0) return raw;
        uint256 bucket = strategy == 4 ? 1 + (raw / 4) % 3 : strategy;
        uint256 offset = (raw / 12) % 9;
        if (bucket == 1) return offset;
        if (bucket == 2) return pivot - 4 + offset;
        return uint256(type(uint32).max) - 8 + offset;
    }
}
