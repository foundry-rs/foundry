//! commonly used sol generated types
use alloy_sol_types::sol;

sol!(
    #[sol(rpc)]
    Greeter,
    "test-data/greeter.json"
);

sol!(
    #[derive(Debug)]
    #[sol(rpc)]
    SimpleStorage,
    "test-data/SimpleStorage.json"
);

sol!(
    #[sol(rpc)]
    Multicall,
    "test-data/multicall.json"
);

sol!(
    #[sol(rpc)]
    contract BUSD {
        function balanceOf(address) external view returns (uint256);
    }
);

sol!(
    #[sol(rpc)]
    interface ERC721 {
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

// https://docs.soliditylang.org/en/latest/control-structures.html#revert
sol!(
// SPDX-License-Identifier: GPL-3.0
pragma solidity ^0.8.4;

#[sol(rpc, bytecode = "6080806040523460155761011e908161001a8239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c9081633ccfd60b146094575063d96a094a14602f575f80fd5b6020366003190112609057671bc16d674ec80000340460043511604e57005b60405162461bcd60e51b815260206004820152601a6024820152792737ba1032b737bab3b41022ba3432b910383937bb34b232b21760311b6044820152606490fd5b5f80fd5b346090575f3660031901126090575f546001600160a01b0316330360da575f8080804781811560d2575b3390f11560c757005b6040513d5f823e3d90fd5b506108fc60be565b6282b42960e81b8152600490fdfea2646970667358221220c143fcbf0da5cee61ae3fcc385d9f7c4d6a7fb2ea42530d70d6049478db0b8a964736f6c63430008190033")]
contract VendingMachine {
    address owner;
    error Unauthorized();
    #[derive(Debug)]
    function buy(uint amount) public payable {
        if (amount > msg.value / 2 ether)
            revert("Not enough Ether provided.");
        // Alternative way to do it:
        require(
            amount <= msg.value / 2 ether,
            "Not enough Ether provided."
        );
        // Perform the purchase.
    }
    function withdraw() public {
        if (msg.sender != owner)
            revert Unauthorized();

        payable(msg.sender).transfer(address(this).balance);
    }
}
);
