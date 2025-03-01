use alloy_primitives::{address, hex, U256};
use alloy_sol_types::{sol, SolCall, SolConstructor, SolInterface};

sol! {
    /// Interface of the ERC20 standard as defined in [the EIP].
    ///
    /// [the EIP]: https://eips.ethereum.org/EIPS/eip-20
    #[derive(Debug, PartialEq, Eq)]
    contract ERC20 {
        mapping(address account => uint256) public balanceOf;

        constructor(string name, string symbol);

        event Transfer(address indexed from, address indexed to, uint256 value);
        event Approval(address indexed owner, address indexed spender, uint256 value);

        function totalSupply() external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
    }
}

#[test]
fn constructor() {
    let constructor_args =
        ERC20::constructorCall::new((String::from("Wrapped Ether"), String::from("WETH")))
            .abi_encode();
    let constructor_args_expected = hex!("00000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000d577261707065642045746865720000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000045745544800000000000000000000000000000000000000000000000000000000");

    assert_eq!(constructor_args.as_slice(), constructor_args_expected);
}

#[test]
fn transfer() {
    // random mainnet ERC20 transfer
    // https://etherscan.io/tx/0x947332ff624b5092fb92e8f02cdbb8a50314e861a4b39c29a286b3b75432165e
    let data = hex!(
        "a9059cbb"
        "0000000000000000000000008bc47be1e3abbaba182069c89d08a61fa6c2b292"
        "0000000000000000000000000000000000000000000000000000000253c51700"
    );
    let expected = ERC20::transferCall {
        to: address!("0x8bc47be1e3abbaba182069c89d08a61fa6c2b292"),
        amount: U256::from(9995360000_u64),
    };

    assert_eq!(data[..4], ERC20::transferCall::SELECTOR);
    let decoded = ERC20::ERC20Calls::abi_decode(&data, true).unwrap();
    assert_eq!(decoded, ERC20::ERC20Calls::transfer(expected));
    assert_eq!(decoded.abi_encode(), data);
}
