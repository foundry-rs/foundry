//! commonly used abigen generated types
use alloy_sol_types::sol;

sol!(
    #[sol(rpc)]
    AlloyGreeter,
    "test-data/greeter.json"
);

sol!(
    #[derive(Debug)]
    #[sol(rpc)]
    AlloySimpleStorage,
    "test-data/SimpleStorage.json"
);

sol!(
    #[sol(rpc)]
    AlloyMulticallContract,
    "test-data/multicall.json"
);

sol!(
    #[sol(rpc)]
    contract AlloyBUSD {
        function balanceOf(address) external view returns (uint256);
    }
);

sol!(
    #[sol(rpc)]
    #[derive(Debug)]
    VendingMachine,
    "test-data/VendingMachine.json"
);

sol!(
    #[sol(rpc)]
    interface AlloyErc721 {
        function balanceOf(address owner) public view virtual returns (uint256);
        function ownerOf(uint256 tokenId) public view virtual returns (address);
        function name() public view virtual returns (string memory);
        function symbol() public view virtual returns (string memory);
        function tokenURI(uint256 tokenId) public view virtual returns (string memory);
        function getApproved(uint256 tokenId) public view virtual returns (address);
        function setApprovalForAll(address operator, bool approved) public virtual;
        function isApprovedForAll(address owner, address operator) public view virtual returns (bool);
        function transferFrom(address from, address to, uint256 tokenId) public virtual;
        function safeTransferFrom(address from, address to, uint256 tokenId) public;
        function safeTransferFrom(address from, address to, uint256 tokenId, bytes memory data) public virtual;
        function _mint(address to, uint256 tokenId) internal;
        function _safeMint(address to, uint256 tokenId, bytes memory data) internal virtual;
        function _burn(uint256 tokenId) internal;
        function _transfer(address from, address to, uint256 tokenId) internal;
        function _approve(address to, uint256 tokenId, address auth) internal;
    }
);
