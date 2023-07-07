interface test {
    event ApplyOwnership(address admin);
    event Approval(address indexed _owner, address indexed _spender, uint256 _value);
    event CommitOwnership(address admin);
    event Deposit(address indexed provider, uint256 value);
    event RewardDataUpdate(address indexed _token, uint256 _amount);
    event Transfer(address indexed _from, address indexed _to, uint256 _value);
    event UpdateLiquidityLimit(
        address user, uint256 original_balance, uint256 original_supply, uint256 working_balance, uint256 working_supply
    );
    event Withdraw(address indexed provider, uint256 value);

    function SDT() external view returns (address);
    function accept_transfer_ownership() external;
    function add_reward(address _reward_token, address _distributor) external;
    function admin() external view returns (address);
    function allowance(address arg0, address arg1) external view returns (uint256);
    function approve(address _spender, uint256 _value) external returns (bool);
    function balanceOf(address arg0) external view returns (uint256);
    function claim_rewards() external;
    function claim_rewards(address _addr) external;
    function claim_rewards(address _addr, address _receiver) external;
    function claim_rewards_for(address _addr, address _receiver) external;
    function claimable_reward(address _user, address _reward_token) external view returns (uint256);
    function claimed_reward(address _addr, address _token) external view returns (uint256);
    function claimer() external view returns (address);
    function commit_transfer_ownership(address addr) external;
    function decimal_staking_token() external view returns (uint256);
    function decimals() external view returns (uint256);
    function decreaseAllowance(address _spender, uint256 _subtracted_value) external returns (bool);
    function deposit(uint256 _value) external;
    function deposit(uint256 _value, address _addr) external;
    function deposit(uint256 _value, address _addr, bool _claim_rewards) external;
    function deposit_reward_token(address _reward_token, uint256 _amount) external;
    function future_admin() external view returns (address);
    function increaseAllowance(address _spender, uint256 _added_value) external returns (bool);
    function initialize(
        address _staking_token,
        address _admin,
        address _SDT,
        address _voting_escrow,
        address _veBoost_proxy,
        address _distributor
    ) external;
    function initialized() external view returns (bool);
    function integrate_checkpoint_of(address arg0) external view returns (uint256);
    function kick(address addr) external;
    function name() external view returns (string memory);
    function reward_count() external view returns (uint256);
    function reward_data(address arg0)
        external
        view
        returns (
            address token,
            address distributor,
            uint256 period_finish,
            uint256 rate,
            uint256 last_update,
            uint256 integral
        );
    function reward_integral_for(address arg0, address arg1) external view returns (uint256);
    function reward_tokens(uint256 arg0) external view returns (address);
    function rewards_receiver(address arg0) external view returns (address);
    function set_claimer(address _claimer) external;
    function set_reward_distributor(address _reward_token, address _distributor) external;
    function set_rewards_receiver(address _receiver) external;
    function staking_token() external view returns (address);
    function symbol() external view returns (string memory);
    function totalSupply() external view returns (uint256);
    function transfer(address _to, uint256 _value) external returns (bool);
    function transferFrom(address _from, address _to, uint256 _value) external returns (bool);
    function user_checkpoint(address addr) external returns (bool);
    function veBoost_proxy() external view returns (address);
    function voting_escrow() external view returns (address);
    function withdraw(uint256 _value) external;
    function withdraw(uint256 _value, bool _claim_rewards) external;
    function working_balances(address arg0) external view returns (uint256);
    function working_supply() external view returns (uint256);
}
