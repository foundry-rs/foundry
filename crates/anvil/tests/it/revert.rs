use crate::abi::VendingMachine;
use alloy_network::TransactionBuilder;
use alloy_primitives::{bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use alloy_sol_types::sol;
use anvil::{spawn, NodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn test_deploy_reverting() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let sender = handle.dev_accounts().next().unwrap();

    let code = bytes!("5f5ffd"); // PUSH0 PUSH0 REVERT
    let tx = TransactionRequest::default().from(sender).with_deploy_code(code);
    let tx = WithOtherFields::new(tx);

    // Calling/estimating gas fails early.
    let err = provider.call(&tx).await.unwrap_err();
    let s = err.to_string();
    assert!(s.contains("execution reverted"), "{s:?}");

    // Sending the transaction is successful but reverts on chain.
    let tx = provider.send_transaction(tx).await.unwrap();
    let receipt = tx.get_receipt().await.unwrap();
    assert!(!receipt.inner.inner.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_revert_messages() {
    sol!(
        #[sol(rpc, bytecode = "608080604052346025575f80546001600160a01b031916600117905560b69081602a8239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c635b9fdc30146023575f80fd5b34607c575f366003190112607c575f546001600160a01b03163303604c576020604051607b8152f35b62461bcd60e51b815260206004820152600b60248201526a08585d5d1a1bdc9a5e995960aa1b6044820152606490fd5b5f80fdfea2646970667358221220f593e5ccd46935f623185de62a72d9f1492d8d15075a111b0fa4d7e16acf4a7064736f6c63430008190033")]
        contract Contract {
            address private owner;

            constructor() {
                owner = address(1);
            }

            modifier onlyOwner() {
                require(msg.sender == owner, "!authorized");
                _;
            }

            #[derive(Debug)]
            function getSecret() public onlyOwner view returns(uint256 secret) {
                return 123;
            }
        }
    );

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let contract = Contract::deploy(&provider).await.unwrap();

    let err = contract.getSecret().call().await.unwrap_err();
    let s = err.to_string();
    assert!(s.contains("!authorized"), "{s:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_solc_revert_example() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let sender = handle.dev_accounts().next().unwrap();
    let provider = handle.http_provider();

    let contract = VendingMachine::deploy(&provider).await.unwrap();

    let err =
        contract.buy(U256::from(100)).value(U256::from(1)).from(sender).call().await.unwrap_err();
    let s = err.to_string();
    assert!(s.contains("Not enough Ether provided."), "{s:?}");
}

// <https://github.com/foundry-rs/foundry/issues/1871>
#[tokio::test(flavor = "multi_thread")]
async fn test_another_revert_message() {
    sol!(
        #[sol(rpc, bytecode = "6080806040523460135760d7908160188239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c9081633fb5c1cb14604d5750638381f58a14602f575f80fd5b346049575f36600319011260495760205f54604051908152f35b5f80fd5b346049576020366003190112604957600435908115606a57505f55005b62461bcd60e51b81526020600482015260126024820152712932bb32b93a29ba3934b733a337b7a130b960711b6044820152606490fdfea2646970667358221220314bf8261cc467619137c071584f8d3bd8d9d97bf2846c138c0567040cf9828a64736f6c63430008190033")]
        contract Contract {
            uint256 public number;

            #[derive(Debug)]
            function setNumber(uint256 num) public {
                require(num != 0, "RevertStringFooBar");
                number = num;
            }
        }
    );

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let contract = Contract::deploy(&provider).await.unwrap();

    let err = contract.setNumber(U256::from(0)).call().await.unwrap_err();
    let s = err.to_string();
    assert!(s.contains("RevertStringFooBar"), "{s:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_solc_revert_custom_errors() {
    sol!(
        #[sol(rpc, bytecode = "608080604052346013576081908160188239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c63e57207e6146023575f80fd5b346047575f3660031901126047576373ea2a7f60e01b815260016004820152602490fd5b5f80fdfea26469706673582212202a8d69545801394af36c56ca229b52ae0b22d7b8f938b107dca8ebbf655464f764736f6c63430008190033")]
        contract Contract {
            error AddressRevert(address);

            #[derive(Debug)]
            function revertAddress() public {
                revert AddressRevert(address(1));
            }
        }
    );

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let contract = Contract::deploy(&provider).await.unwrap();

    let err = contract.revertAddress().call().await.unwrap_err();
    let s = err.to_string();
    assert!(s.contains("execution reverted"), "{s:?}");
}
