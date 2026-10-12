//! Solidity targeting hooks used by invariant campaigns.

use alloy_sol_types::sol;

sol! {
    interface IInvariantTest {
        #[derive(Default)]
        struct FuzzSelector {
            address addr;
            bytes4[] selectors;
        }

        #[derive(Default)]
        struct FuzzArtifactSelector {
            string artifact;
            bytes4[] selectors;
        }

        #[derive(Default)]
        struct FuzzInterface {
            address addr;
            string[] artifacts;
        }

        function afterInvariant() external;

        #[derive(Default)]
        function excludeArtifacts() public view returns (string[] memory excludedArtifacts);

        #[derive(Default)]
        function excludeContracts() public view returns (address[] memory excludedContracts);

        #[derive(Default)]
        function excludeSelectors() public view returns (FuzzSelector[] memory excludedSelectors);

        #[derive(Default)]
        function excludeSenders() public view returns (address[] memory excludedSenders);

        #[derive(Default)]
        function targetArtifacts() public view returns (string[] memory targetedArtifacts);

        #[derive(Default)]
        function targetArtifactSelectors() public view returns (FuzzArtifactSelector[] memory targetedArtifactSelectors);

        #[derive(Default)]
        function targetContracts() public view returns (address[] memory targetedContracts);

        #[derive(Default)]
        function targetSelectors() public view returns (FuzzSelector[] memory targetedSelectors);

        #[derive(Default)]
        function targetSenders() public view returns (address[] memory targetedSenders);

        #[derive(Default)]
        function targetInterfaces() public view returns (FuzzInterface[] memory targetedInterfaces);
    }
}
