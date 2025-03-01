//! Multicall module to organize and implement required functionality for enabling multicall in
//! alloy_provider plus related tests. This avoids cyclic deps between alloy_provider and
//! alloy_contract.
//!
//! This module is not public API.
use super::SolCallBuilder;
use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{Address, Bytes};
use alloy_provider::{MulticallItem, Provider};
use alloy_sol_types::SolCall;

impl<T, P: Provider<N>, C: SolCall, N: Network> MulticallItem for SolCallBuilder<T, P, C, N> {
    type Decoder = C;

    fn target(&self) -> Address {
        self.request.to().expect("`to` not set for the `SolCallBuilder`")
    }

    fn input(&self) -> Bytes {
        self.calldata().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256, U256};
    use alloy_provider::{
        CallItem, CallItemBuilder, Failure, MulticallBuilder, Provider, ProviderBuilder,
    };
    use alloy_sol_types::sol;
    use DummyThatFails::DummyThatFailsInstance;

    sol! {
        #[derive(Debug, PartialEq)]
        #[sol(rpc)]
        interface ERC20 {
            function totalSupply() external view returns (uint256 totalSupply);
            function balanceOf(address owner) external view returns (uint256 balance);
            function transfer(address to, uint256 value) external returns (bool);
        }
    }

    sol! {
        // solc 0.8.25; solc DummyThatFails.sol --optimize --bin
        #[sol(rpc, bytecode = "6080604052348015600e575f80fd5b5060a780601a5f395ff3fe6080604052348015600e575f80fd5b50600436106030575f3560e01c80630b93381b146034578063a9cc4718146036575b5f80fd5b005b603460405162461bcd60e51b815260040160689060208082526004908201526319985a5b60e21b604082015260600190565b60405180910390fdfea2646970667358221220c90ee107375422bb3516f4f13cdd754387c374edb5d9815fb6aa5ca111a77cb264736f6c63430008190033")]
        #[derive(Debug)]
        contract DummyThatFails {
            function fail() external {
                revert("fail");
            }

            function success() external {}
        }
    }

    async fn deploy_dummy(
        provider: impl alloy_provider::Provider,
    ) -> DummyThatFailsInstance<(), impl alloy_provider::Provider> {
        DummyThatFails::deploy(provider).await.unwrap()
    }

    const FORK_URL: &str = "https://eth-mainnet.alchemyapi.io/v2/jGiK5vwDfC3F4r0bqukm-W2GqgdrxdSr";

    #[tokio::test]
    async fn test_single() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let provider = ProviderBuilder::new().on_anvil_with_config(|a| a.fork(FORK_URL));

        let erc20 = ERC20::new(weth, &provider);
        let multicall = provider.multicall().add(erc20.totalSupply());

        let (_total_supply,) = multicall.aggregate().await.unwrap();
    }

    #[tokio::test]
    async fn test_aggregate() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let provider = ProviderBuilder::new().on_anvil_with_config(|a| a.fork(FORK_URL));

        let erc20 = ERC20::new(weth, &provider);

        let multicall = provider
            .multicall()
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")))
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")));

        let (t1, b1, t2, b2) = multicall.aggregate().await.unwrap();

        assert_eq!(t1, t2);
        assert_eq!(b1, b2);
    }

    #[tokio::test]
    async fn test_try_aggregate_pass() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let provider = ProviderBuilder::new().on_anvil_with_config(|a| a.fork(FORK_URL));
        let erc20 = ERC20::new(weth, &provider);

        let multicall = provider
            .multicall()
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")))
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")));

        let (_t1, _b1, _t2, _b2) = multicall.try_aggregate(true).await.unwrap();
    }

    #[tokio::test]
    async fn aggregate3() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

        let provider =
            ProviderBuilder::new().on_anvil_with_wallet_and_config(|a| a.fork(FORK_URL)).unwrap();

        let dummy = deploy_dummy(provider.clone()).await;
        let erc20 = ERC20::new(weth, &provider);
        let multicall = provider
            .multicall()
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")))
            .add(dummy.fail()); // Failing call that will revert the multicall.

        let err = multicall.aggregate3().await.unwrap_err();

        assert!(err.to_string().contains("revert: Multicall3: call failed"));

        let failing_call = CallItemBuilder::new(dummy.fail()).allow_failure(true);
        let multicall = provider
            .multicall()
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")))
            .add_call(failing_call);
        let (t1, b1, failure) = multicall.aggregate3().await.unwrap();

        assert!(t1.is_ok());
        assert!(b1.is_ok());
        let err = failure.unwrap_err();
        assert!(matches!(err, Failure { idx: 2, return_data: _ }));
    }

    #[tokio::test]
    async fn test_try_aggregate_fail() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let provider =
            ProviderBuilder::new().on_anvil_with_wallet_and_config(|a| a.fork(FORK_URL)).unwrap();

        let dummy_addr = deploy_dummy(provider.clone()).await;
        let erc20 = ERC20::new(weth, &provider);
        let multicall = provider
            .multicall()
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")))
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")))
            .add(dummy_addr.fail());

        let err = multicall.try_aggregate(true).await.unwrap_err();

        assert!(err.to_string().contains("revert: Multicall3: call failed"));

        let (t1, b1, t2, b2, failure) = multicall.try_aggregate(false).await.unwrap();

        assert!(t1.is_ok());
        assert!(b1.is_ok());
        assert!(t2.is_ok());
        assert!(b2.is_ok());
        let err = failure.unwrap_err();
        assert!(matches!(err, Failure { idx: 4, return_data: _ }));
    }

    #[tokio::test]
    async fn test_util() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let provider = ProviderBuilder::new()
            .on_anvil_with_config(|a| a.fork(FORK_URL).fork_block_number(21787144));
        let erc20 = ERC20::new(weth, &provider);
        let multicall = provider
            .multicall()
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")))
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")))
            .get_block_hash(21787144);

        let (t1, b1, t2, b2, block_hash) = multicall.aggregate().await.unwrap();

        assert_eq!(t1, t2);
        assert_eq!(b1, b2);
        assert_eq!(
            block_hash.blockHash,
            b256!("31be03d4fb9a280d1699f1004f340573cd6d717dae79095d382e876415cb26ba")
        );
    }

    sol! {
        // solc 0.8.25; solc PayableCounter.sol --optimize --bin
        #[sol(rpc, bytecode = "6080604052348015600e575f80fd5b5061012c8061001c5f395ff3fe6080604052600436106025575f3560e01c806361bc221a146029578063d09de08a14604d575b5f80fd5b3480156033575f80fd5b50603b5f5481565b60405190815260200160405180910390f35b60536055565b005b5f341160bc5760405162461bcd60e51b815260206004820152602c60248201527f50617961626c65436f756e7465723a2076616c7565206d75737420626520677260448201526b06561746572207468616e20360a41b606482015260840160405180910390fd5b60015f8082825460cb919060d2565b9091555050565b8082018082111560f057634e487b7160e01b5f52601160045260245ffd5b9291505056fea264697066735822122064d656316647d3dc48d7ef0466bd10bc87694802a673183058725926a5190a5564736f6c63430008190033")]
        #[derive(Debug)]
        contract PayableCounter {
            uint256 public counter;

            function increment() public payable {
                require(msg.value > 0, "PayableCounter: value must be greater than 0");
                counter += 1;
            }
        }
    }

    #[tokio::test]
    async fn aggregate3_value() {
        let provider =
            ProviderBuilder::new().on_anvil_with_wallet_and_config(|a| a.fork(FORK_URL)).unwrap();

        let payable_counter = PayableCounter::deploy(provider.clone()).await.unwrap();

        let increment_call = CallItem::<PayableCounter::incrementCall>::new(
            payable_counter.increment().target(),
            payable_counter.increment().input(),
        )
        .value(U256::from(100));

        let multicall = provider
            .multicall()
            .add(payable_counter.counter())
            .add_call(increment_call)
            .add(payable_counter.counter());

        let (c1, inc, c2) = multicall.aggregate3_value().await.unwrap();

        assert_eq!(c1.unwrap().counter, U256::ZERO);
        assert!(inc.is_ok());
        assert_eq!(c2.unwrap().counter, U256::from(1));

        // Allow failure - due to no value being sent
        let increment_call = CallItem::<PayableCounter::incrementCall>::new(
            payable_counter.increment().target(),
            payable_counter.increment().input(),
        )
        .allow_failure(true);

        let multicall = provider
            .multicall()
            .add(payable_counter.counter())
            .add_call(increment_call)
            .add(payable_counter.counter());

        let (c1, inc, c2) = multicall.aggregate3_value().await.unwrap();

        assert_eq!(c1.unwrap().counter, U256::ZERO);
        assert!(inc.is_err_and(|failure| matches!(failure, Failure { idx: 1, return_data: _ })));
        assert_eq!(c2.unwrap().counter, U256::ZERO);
    }

    #[tokio::test]
    async fn test_clear() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let provider = ProviderBuilder::new().on_anvil();

        let erc20 = ERC20::new(weth, &provider);
        let multicall = provider
            .multicall()
            .add(erc20.totalSupply())
            .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045")));
        assert_eq!(multicall.len(), 2);
        let multicall = multicall.clear();
        assert_eq!(multicall.len(), 0);
    }

    #[tokio::test]
    async fn add_dynamic() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let provider = ProviderBuilder::new().on_anvil_with_config(|a| a.fork(FORK_URL));

        let erc20 = ERC20::new(weth, &provider);

        let multicall = MulticallBuilder::new_dynamic(provider.clone())
            .add_dynamic(erc20.totalSupply())
            // .add(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045"))) - WON'T
            // COMPILE
            // .add_dynamic(erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045"))) -
            // WON'T COMPILE
            .add_dynamic(erc20.totalSupply())
            .extend(vec![erc20.totalSupply(), erc20.totalSupply()]);

        let res: Vec<ERC20::totalSupplyReturn> = multicall.aggregate().await.unwrap();

        assert_eq!(res.len(), 4);
        assert_eq!(res[0], res[1]);
    }

    #[tokio::test]
    async fn test_extend_dynamic() {
        let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let provider = ProviderBuilder::new().on_anvil_with_config(|a| a.fork(FORK_URL));
        let erc20 = ERC20::new(weth, &provider);
        let ts_calls = vec![erc20.totalSupply(); 18];
        let multicall = MulticallBuilder::new_dynamic(provider.clone()).extend(ts_calls);

        assert_eq!(multicall.len(), 18);
        let res = multicall.aggregate().await.unwrap();
        assert_eq!(res.len(), 18);
    }
}
