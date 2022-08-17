interface test {
    event AuctionEnded(uint128 indexed auction_number);
    event AuctionStarted(uint128 indexed auction_number);
    event AuctionStarterSet(address indexed starter);
    event AutopayBatchSizeSet(uint16 batch_size);
    event BidAdded(address bidder, address indexed validator, address indexed opportunity, uint256 amount, uint256 indexed auction_number);
    event BidTokenSet(address indexed token);
    event FastLaneFeeSet(uint256 amount);
    event MinimumAutoshipThresholdSet(uint128 amount);
    event MinimumBidIncrementSet(uint256 amount);
    event OpportunityAddressDisabled(address indexed opportunity, uint128 indexed auction_number);
    event OpportunityAddressEnabled(address indexed opportunity, uint128 indexed auction_number);
    event OpsSet(address ops);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event PausedStateSet(bool state);
    event ResolverMaxGasPriceSet(uint128 amount);
    event ValidatorAddressDisabled(address indexed validator, uint128 indexed auction_number);
    event ValidatorAddressEnabled(address indexed validator, uint128 indexed auction_number);
    event ValidatorPreferencesSet(address indexed validator, uint256 minAutoshipAmount, address validatorPayableAddress);
    event ValidatorWithdrawnBalance(address indexed validator, uint128 indexed auction_number, uint256 amount, address destination, address indexed caller);
    event WithdrawStuckERC20(address indexed receiver, address indexed token, uint256 amount);
    event WithdrawStuckNativeToken(address indexed receiver, uint256 amount);

    struct Bid { address a; address b; address c; address d; uint256 e; }
    struct Status { uint128 a; uint128 b; uint8 c; }
    struct ValidatorBalanceCheckpoint { uint256 a; uint256 b; uint128 c; uint128 d; }
    struct ValidatorPreferences { uint256 a; address b; }

    function MAX_AUCTION_VALUE() view external returns (uint128);
    function auctionStarter() view external returns (address);
    function auction_live() view external returns (bool);
    function auction_number() view external returns (uint128);
    function autopay_batch_size() view external returns (uint16);
    function bid_increment() view external returns (uint256);
    function bid_token() view external returns (address);
    function checker() view external returns (bool canExec, bytes memory execPayload);
    function disableOpportunityAddress(address opportunityAddress) external;
    function disableValidatorAddress(address _validatorAddress) external;
    function enableOpportunityAddress(address opportunityAddress) external;
    function enableValidatorAddress(address _validatorAddress) external;
    function enableValidatorAddressWithPreferences(address _validatorAddress, uint128 _minAutoshipAmount, address _validatorPayableAddress) external;
    function endAuction() external returns (bool);
    function fast_lane_fee() view external returns (uint24);
    function findFinalizedAuctionWinnerAtAuction(uint128 auction_index, address validatorAddress, address opportunityAddress) view external returns (bool, address, uint128);
    function findLastFinalizedAuctionWinner(address validatorAddress, address opportunityAddress) view external returns (bool, address, uint128);
    function findLiveAuctionTopBid(address validatorAddress, address opportunityAddress) view external returns (uint256, uint128);
    function getActivePrivilegesAuctionNumber() view external returns (uint128);
    function getAutopayJobs(uint16 batch_size, uint128 auction_index) view external returns (bool hasJobs, address[] memory autopayRecipients);
    function getCheckpoint(address who) view external returns (ValidatorBalanceCheckpoint memory);
    function getPreferences(address who) view external returns (ValidatorPreferences memory);
    function getStatus(address who) view external returns (Status memory);
    function getValidatorsActiveAtAuction(uint128 auction_index) view external returns (address[] memory);
    function init(address _initial_bid_token, address _ops, address _starter) external;
    function max_gas_price() view external returns (uint128);
    function minAutoShipThreshold() view external returns (uint128);
    function ops() view external returns (address);
    function outstandingFLBalance() view external returns (uint256);
    function owner() view external returns (address);
    function processAutopayJobs(address[] memory autopayRecipients) external;
    function redeemOutstandingBalance(address outstandingValidatorWithBalance) external;
    function renounceOwnership() external;
    function setAutopayBatchSize(uint16 size) external;
    function setBidToken(address _bid_token_address) external;
    function setFastlaneFee(uint24 _fastLaneFee) external;
    function setMinimumAutoShipThreshold(uint128 _minAmount) external;
    function setMinimumBidIncrement(uint256 _bid_increment) external;
    function setOffchainCheckerDisabledState(bool state) external;
    function setOps(address _ops) external;
    function setPausedState(bool state) external;
    function setResolverMaxGasPrice(uint128 _maxgas) external;
    function setStarter(address _starter) external;
    function setValidatorPreferences(uint128 _minAutoshipAmount, address _validatorPayableAddress) external;
    function startAuction() external;
    function submitBid(Bid memory bid) external;
    function transferOwnership(address newOwner) external;
    function withdrawStuckERC20(address _tokenAddress) external;
    function withdrawStuckNativeToken(uint256 amount) external;
}
