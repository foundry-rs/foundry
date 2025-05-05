use alloy_sol_types::sol;

sol! {
    /// ENS Registry contract.
    #[sol(rpc)]
    contract EnsRegistry {
        /// Sets subnode record
        function setSubnodeRecord(
                bytes32 node,
                bytes32 label,
                address owner,
                address resolver,
                uint64 ttl
            ) external;
    }

    /// ENS Name Wrapper contract
    #[sol(rpc)]
    contract NameWrapper {
        function isWrapped(bytes32 node) external returns (bool);
        function setSubnodeRecord(
                bytes32 node,
                string label,
                address owner,
                address resolver,
                uint64 ttl,
                uint32 fuses,
                uint64 expiry,
            ) external;
    }

    /// ENS Public Resolver contract
    #[sol(rpc)]
    contract PublicResolver {
        function setAddr(bytes32 node, address addr) external;
        function addr(bytes32 node) external returns (address);
        function setName(bytes32 node,  string newName) external;
    }

    /// ENS Reverse Registrar contract
    #[sol(rpc)]
    contract ReverseRegistrar {
        function setName(string memory name) external returns (bytes32);
        function setNameForAddr(address addr, address owner, address resolver, string name) external;
    }
}
