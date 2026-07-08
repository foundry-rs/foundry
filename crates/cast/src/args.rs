use crate::{
    Cast, SimpleCast,
    cmd::erc20::IERC20,
    opts::{Cast as CastArgs, CastSubcommand, ToBaseArgs},
    tempo::tempo_provider,
    traces::identifier::SignaturesIdentifier,
    tx::CastTxSender,
};
use alloy_dyn_abi::{ErrorExt, EventExt};
use alloy_eips::eip7702::SignedAuthorization;
use alloy_ens::{ProviderEnsExt, namehash};
use alloy_network::{Ethereum, eip2718::Decodable2718};
use alloy_primitives::{Address, B256, eip191_hash_message, hex, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag::Latest};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use eyre::{Result, WrapErr};
use foundry_cli::{
    json::{print_json_object, print_json_value_or_scalar, print_list, print_scalar, print_tokens},
    utils::{self, LoadConfig},
};
use foundry_common::{
    abi::{get_error, get_event},
    fmt::format_uint_exp,
    fs,
    provider::ProviderBuilder,
    selectors::{
        ParsedSignatures, SelectorImportData, SelectorKind, decode_calldata, decode_event_topic,
        decode_function_selector, decode_selectors, import_selectors, parse_signatures,
        pretty_calldata,
    },
    shell, stdin,
    tempo::{PaymentLaneClassification, classify_payment_lane},
};
use foundry_evm_networks::NetworkVariant;
use foundry_primitives::{FoundryNetwork, FoundryTxEnvelope};
#[cfg(feature = "optimism")]
use op_alloy_network::Optimism;
use std::time::Instant;
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{ITIP20ChannelReserve, TIP20_CHANNEL_RESERVE_ADDRESS};

/// Run the `cast` command-line interface.
pub fn run() -> Result<()> {
    foundry_cli::opts::GlobalArgs::check_markdown_help::<CastArgs>();

    setup()?;

    let args = CastArgs::parse();
    args.global.init()?;
    args.global.tokio_runtime().block_on(run_command(args))
}

/// Setup the global logger and other utilities.
pub fn setup() -> Result<()> {
    utils::common_setup();
    utils::subscriber();

    Ok(())
}

/// Run the subcommand.
#[allow(clippy::large_stack_frames)]
pub async fn run_command(args: CastArgs) -> Result<()> {
    match args.cmd {
        // Constants
        CastSubcommand::MaxInt { r#type } => {
            let out = SimpleCast::max_int(&r#type)?;
            print_scalar(out)?;
        }
        CastSubcommand::MinInt { r#type } => {
            let out = SimpleCast::min_int(&r#type)?;
            print_scalar(out)?;
        }
        CastSubcommand::MaxUint { r#type } => {
            let out = SimpleCast::max_int(&r#type)?;
            print_scalar(out)?;
        }
        CastSubcommand::AddressZero => {
            print_scalar(format!("{:?}", Address::ZERO))?;
        }
        CastSubcommand::HashZero => {
            print_scalar(format!("{:?}", B256::ZERO))?;
        }

        // Conversions & transformations
        CastSubcommand::FromUtf8 { text } => {
            let value = stdin::unwrap(text, false)?;
            let out = SimpleCast::from_utf8(&value);
            print_scalar(out)?;
        }
        CastSubcommand::ToAscii { hexdata } => {
            let value = stdin::unwrap(hexdata, false)?;
            let out = SimpleCast::to_ascii(value.trim())?;
            print_scalar(out)?;
        }
        CastSubcommand::ToUtf8 { hexdata } => {
            let value = stdin::unwrap(hexdata, false)?;
            let out = SimpleCast::to_utf8(&value)?;
            print_scalar(out)?;
        }
        CastSubcommand::FromFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;
            let out = SimpleCast::from_fixed_point(&value, &decimals)?;
            print_scalar(out)?;
        }
        CastSubcommand::ToFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;
            let out = SimpleCast::to_fixed_point(&value, &decimals)?;
            print_scalar(out)?;
        }
        CastSubcommand::ConcatHex { data } => {
            let out = if data.is_empty() {
                let s = stdin::read(true)?;
                SimpleCast::concat_hex(s.split_whitespace())
            } else {
                SimpleCast::concat_hex(data)
            };
            print_scalar(out)?;
        }
        CastSubcommand::FromBin => {
            let hex = stdin::read_bytes(false)?;
            let out = hex::encode_prefixed(hex);
            print_scalar(out)?;
        }
        CastSubcommand::ToHexdata { input } => {
            let value = stdin::unwrap_line(input)?;
            let output = match value {
                s if s.starts_with('@') => hex::encode(std::env::var(&s[1..])?),
                s if s.starts_with('/') => hex::encode(fs::read(s)?),
                s => s.split(':').map(|s| s.trim_start_matches("0x").to_lowercase()).collect(),
            };
            print_scalar(format!("0x{output}"))?;
        }
        CastSubcommand::ToCheckSumAddress { address, chain_id } => {
            let value = stdin::unwrap_line(address)?;
            let out = value.to_checksum(chain_id);
            print_scalar(out)?;
        }
        CastSubcommand::ToUint256 { value } => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::to_uint256(&value)?;
            print_scalar(out)?;
        }
        CastSubcommand::ToInt256 { value } => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::to_int256(&value)?;
            print_scalar(out)?;
        }
        CastSubcommand::ToUnit { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::to_unit(&value, &unit)?;
            print_scalar(out)?;
        }
        CastSubcommand::ParseUnits { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::parse_units(&value, unit)?;
            print_scalar(out)?;
        }
        CastSubcommand::FormatUnits { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::format_units(&value, unit)?;
            print_scalar(out)?;
        }
        CastSubcommand::FromWei { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::from_wei(&value, &unit)?;
            print_scalar(out)?;
        }
        CastSubcommand::ToWei { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::to_wei(&value, &unit)?;
            print_scalar(out)?;
        }
        CastSubcommand::FromRlp { value, as_int } => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::from_rlp(value, as_int)?;
            print_scalar(out)?;
        }
        CastSubcommand::ToRlp { value } => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::to_rlp(&value)?;
            print_scalar(out)?;
        }
        CastSubcommand::ToHex(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::to_base(&value, base_in.as_deref(), "hex")?;
            print_scalar(out)?;
        }
        CastSubcommand::ToDec(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            let out = SimpleCast::to_base(&value, base_in.as_deref(), "dec")?;
            print_scalar(out)?;
        }
        CastSubcommand::ToBase { base: ToBaseArgs { value, base_in }, base_out } => {
            let (value, base_out) = stdin::unwrap2(value, base_out)?;
            let out = SimpleCast::to_base(&value, base_in.as_deref(), &base_out)?;
            print_scalar(out)?;
        }
        CastSubcommand::ToBytes32 { bytes } => {
            let value = stdin::unwrap_line(bytes)?;
            let out = SimpleCast::to_bytes32(&value)?;
            print_scalar(out)?;
        }
        CastSubcommand::Pad { data, right, left: _, len } => {
            let value = stdin::unwrap_line(data)?;
            let out = SimpleCast::pad(&value, right, len)?;
            print_scalar(out)?;
        }
        CastSubcommand::FormatBytes32String { string } => {
            let value = stdin::unwrap_line(string)?;
            let out = SimpleCast::format_bytes32_string(&value)?;
            print_scalar(out)?;
        }
        CastSubcommand::ParseBytes32String { bytes } => {
            let value = stdin::unwrap_line(bytes)?;
            let out = SimpleCast::parse_bytes32_string(&value)?;
            print_scalar(out)?;
        }
        CastSubcommand::ParseBytes32Address { bytes } => {
            let value = stdin::unwrap_line(bytes)?;
            let out = SimpleCast::parse_bytes32_address(&value)?;
            print_scalar(out)?;
        }

        // ABI encoding & decoding
        CastSubcommand::DecodeAbi { sig, calldata, input } => {
            let tokens = SimpleCast::abi_decode(&sig, &calldata, input)?;
            print_tokens(&tokens)?;
        }
        CastSubcommand::AbiEncode { sig, packed, args } => {
            let out = if packed {
                SimpleCast::abi_encode_packed(&sig, &args)?
            } else {
                SimpleCast::abi_encode(&sig, &args)?
            };
            print_scalar(out)?;
        }
        // TODO(json): multi-line output (one line per topic + data field), needs structured object
        // envelope
        CastSubcommand::AbiEncodeEvent { sig, args } => {
            let log_data = SimpleCast::abi_encode_event(&sig, &args)?;
            if shell::is_json() {
                #[derive(serde::Serialize)]
                struct EncodedEvent {
                    topics: Vec<String>,
                    data: String,
                }
                let encoded = EncodedEvent {
                    topics: log_data.topics().iter().map(|t| t.to_string()).collect(),
                    data: hex::encode_prefixed(&log_data.data),
                };
                print_json_object(encoded)?;
            } else {
                for (i, topic) in log_data.topics().iter().enumerate() {
                    sh_println!("[topic{}]: {}", i, topic)?;
                }
                if !log_data.data.is_empty() {
                    sh_println!("[data]: {}", hex::encode_prefixed(log_data.data))?;
                }
            }
        }
        CastSubcommand::DecodeCalldata { sig, calldata, file } => {
            let raw_hex = if let Some(file_path) = file {
                let contents = fs::read_to_string(&file_path)?;
                contents.trim().to_string()
            } else {
                calldata.unwrap()
            };

            let tokens = SimpleCast::calldata_decode(&sig, &raw_hex, true)?;
            print_tokens(&tokens)?;
        }
        CastSubcommand::CalldataEncode { sig, args, file } => {
            let final_args = if let Some(file_path) = file {
                let contents = fs::read_to_string(file_path)?;
                contents
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(String::from)
                    .collect()
            } else {
                args
            };
            let out = SimpleCast::calldata_encode(sig, &final_args)?;
            print_scalar(out)?;
        }
        CastSubcommand::DecodeString { data } => {
            let tokens = SimpleCast::calldata_decode("Any(string)", &data, true)?;
            print_tokens(&tokens)?;
        }
        CastSubcommand::DecodeEvent { sig, data } => {
            let decoded_event = if let Some(event_sig) = sig {
                let event = get_event(event_sig.as_str())?;
                event.decode_log_parts(core::iter::once(event.selector()), &hex::decode(data)?)?
            } else {
                let data = crate::strip_0x(&data);
                let selector = data.get(..64).unwrap_or_default();
                let selector = selector.parse()?;
                let identified_event =
                    SignaturesIdentifier::new(false)?.identify_event(selector).await;
                if let Some(event) = identified_event {
                    let _ = sh_println!("{}", event.signature());
                    let data = data.get(64..).unwrap_or_default();
                    get_event(event.signature().as_str())?
                        .decode_log_parts(core::iter::once(selector), &hex::decode(data)?)?
                } else {
                    eyre::bail!("No matching event signature found for selector `{selector}`")
                }
            };
            print_tokens(&decoded_event.body)?;
        }
        CastSubcommand::DecodeError { sig, data } => {
            let error = if let Some(err_sig) = sig {
                get_error(err_sig.as_str())?
            } else {
                let data = crate::strip_0x(&data);
                let selector = data.get(..8).unwrap_or_default();
                let identified_error =
                    SignaturesIdentifier::new(false)?.identify_error(selector.parse()?).await;
                if let Some(error) = identified_error {
                    let _ = sh_println!("{}", error.signature());
                    error
                } else {
                    eyre::bail!("No matching error signature found for selector `{selector}`")
                }
            };
            let decoded_error = error.decode_error(&hex::decode(data)?)?;
            print_tokens(&decoded_error.body)?;
        }
        CastSubcommand::Interface(cmd) => cmd.run().await?,
        CastSubcommand::CreationCode(cmd) => cmd.run().await?,
        CastSubcommand::ConstructorArgs(cmd) => cmd.run().await?,
        CastSubcommand::Artifact(cmd) => cmd.run().await?,
        CastSubcommand::Bind(cmd) => cmd.run().await?,
        CastSubcommand::B2EPayload(cmd) => cmd.run().await?,
        CastSubcommand::PrettyCalldata { calldata, offline } => {
            let calldata = stdin::unwrap_line(calldata)?;
            let out = pretty_calldata(&calldata, offline).await?.to_string();
            print_scalar(out)?;
        }
        // JSON: --optimize conflicts with --json at the clap level; optimize=None uses print_scalar
        CastSubcommand::Sig { sig, optimize } => {
            let sig = stdin::unwrap_line(sig)?;
            match optimize {
                Some(opt) => {
                    sh_status!("Starting to optimize signature...")?;
                    let start_time = Instant::now();
                    let (selector, signature) = SimpleCast::get_selector(&sig, opt)?;
                    sh_status!("Successfully generated in {:?}", start_time.elapsed())?;
                    sh_println!("Selector: {selector}")?;
                    sh_println!("Optimized signature: {signature}")?;
                }
                None => {
                    let out = SimpleCast::get_selector(&sig, 0)?.0;
                    print_scalar(out)?;
                }
            }
        }

        // Blockchain & RPC queries
        CastSubcommand::AccessList(cmd) => cmd.run().await?,
        CastSubcommand::Age { block, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let out = format!(
                "{} UTC",
                Cast::new(provider).age(block.unwrap_or(BlockId::Number(Latest))).await?
            );
            print_scalar(out)?;
        }
        CastSubcommand::Balance { block, who, ether, rpc, erc20 } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let account_addr = who.resolve(&provider).await?;

            match erc20 {
                Some(token) => {
                    let balance = IERC20::new(token, &provider)
                        .balanceOf(account_addr)
                        .block(block.unwrap_or_default())
                        .call()
                        .await?;

                    sh_warn!("--erc20 flag is deprecated, use `cast erc20 balance` instead")?;
                    print_scalar(format_uint_exp(balance))?;
                }
                None => {
                    let value = Cast::new(&provider).balance(account_addr, block).await?;
                    let out = if ether {
                        SimpleCast::from_wei(&value.to_string(), "eth")?
                    } else {
                        value.to_string()
                    };
                    print_scalar(out)?;
                }
            }
        }
        CastSubcommand::BaseFee { block, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let out = Cast::new(provider)
                .base_fee(block.unwrap_or(BlockId::Number(Latest)))
                .await?
                .to_string();
            print_scalar(out)?;
        }
        CastSubcommand::Block { block, full, fields, raw, rpc, network } => {
            let config = rpc.load_config()?;
            // Can use either --raw or specify raw as a field
            let is_raw_block = raw || fields.contains(&"raw".into());
            let output = if is_raw_block {
                match network {
                    #[cfg(feature = "optimism")]
                    Some(NetworkVariant::Optimism) => {
                        let provider =
                            ProviderBuilder::<Optimism>::from_config(&config)?.build()?;

                        Cast::new(&provider)
                            .block_raw(block.unwrap_or(BlockId::Number(Latest)), full)
                            .await?
                    }
                    Some(NetworkVariant::Tempo) => {
                        let provider = tempo_provider(&config)?;
                        Cast::new(&provider)
                            .block_raw(block.unwrap_or(BlockId::Number(Latest)), full)
                            .await?
                    }
                    // Ethereum (default) or no --raw flag
                    _ => {
                        let provider =
                            ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
                        Cast::new(&provider)
                            .block_raw(block.unwrap_or(BlockId::Number(Latest)), full)
                            .await?
                    }
                }
            } else {
                let provider = utils::get_provider(&config)?;
                Cast::new(provider)
                    .block(block.unwrap_or(BlockId::Number(Latest)), full, fields)
                    .await?
            };
            print_json_value_or_scalar(output)?;
        }
        CastSubcommand::BlockNumber { rpc, block } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let number = match block {
                Some(id) => {
                    provider
                        .get_block(id)
                        .await?
                        .ok_or_else(|| eyre::eyre!("block {id:?} not found"))?
                        .header
                        .number
                }
                None => Cast::new(provider).block_number().await?,
            };
            print_scalar(number)?;
        }
        CastSubcommand::Chain { rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let out = Cast::new(provider).chain().await?.to_string();
            print_scalar(out)?;
        }
        CastSubcommand::ChainId { rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let out = Cast::new(provider).chain_id().await?.to_string();
            print_scalar(out)?;
        }
        CastSubcommand::Client { rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let out = provider.get_client_version().await?;
            print_scalar(out)?;
        }
        CastSubcommand::Code { block, who, disassemble, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            let out = Cast::new(provider).code(who, block, disassemble).await?;
            print_scalar(out)?;
        }
        CastSubcommand::Codesize { block, who, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            let out = Cast::new(provider).codesize(who, block).await?;
            print_scalar(out)?;
        }
        CastSubcommand::ComputeAddress { address, nonce, salt, init_code, init_code_hash, rpc } => {
            let address = stdin::unwrap_line(address)?;
            let computed = {
                // For CREATE2, init_code_hash is needed to compute the address
                if let Some(init_code_hash) = init_code_hash {
                    address.create2(salt.unwrap_or(B256::ZERO), init_code_hash)
                } else if let Some(init_code) = init_code {
                    address.create2(salt.unwrap_or(B256::ZERO), keccak256(hex::decode(init_code)?))
                } else {
                    // For CREATE, rpc is needed to compute the address
                    let config = rpc.load_config()?;
                    let provider = utils::get_provider(&config)?;
                    Cast::new(provider).compute_address(address, nonce).await?
                }
            };
            let addr = computed.to_checksum(None);
            print_scalar(addr)?;
        }
        CastSubcommand::Disassemble { bytecode } => {
            let bytecode = stdin::unwrap_line(bytecode)?;
            let out = SimpleCast::disassemble(&hex::decode(bytecode)?)?;
            print_scalar(out)?;
        }
        CastSubcommand::Selectors { bytecode, resolve } => {
            let bytecode = stdin::unwrap_line(bytecode)?;
            let functions = SimpleCast::extract_functions(&bytecode)?;

            let resolve_results: Vec<String> = if resolve {
                let selectors = functions
                    .iter()
                    .map(|&(selector, ..)| SelectorKind::Function(selector))
                    .collect::<Vec<_>>();
                let ds = decode_selectors(&selectors).await?;
                ds.into_iter().map(|v| v.join("|")).collect()
            } else {
                vec![]
            };

            if shell::is_json() {
                #[derive(serde::Serialize)]
                struct SelectorInfo {
                    selector: String,
                    arguments: String,
                    state_mutability: String,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    resolved: Option<String>,
                }
                let infos: Vec<SelectorInfo> = functions
                    .into_iter()
                    .enumerate()
                    .map(|(pos, (selector, arguments, state_mutability))| SelectorInfo {
                        selector: selector.to_string(),
                        arguments,
                        state_mutability: state_mutability.to_string(),
                        resolved: resolve_results.get(pos).cloned(),
                    })
                    .collect();
                print_json_object(infos)?;
            } else {
                let max_args_len = functions.iter().map(|r| r.1.len()).max().unwrap_or(0);
                let max_mutability_len = functions.iter().map(|r| r.2.len()).max().unwrap_or(0);
                for (pos, (selector, arguments, state_mutability)) in
                    functions.into_iter().enumerate()
                {
                    if resolve {
                        let resolved = &resolve_results[pos];
                        sh_println!(
                            "{selector}\t{arguments:max_args_len$}\t{state_mutability:max_mutability_len$}\t{resolved}"
                        )?
                    } else {
                        sh_println!("{selector}\t{arguments:max_args_len$}\t{state_mutability}")?
                    }
                }
            }
        }
        CastSubcommand::FindBlock(cmd) => cmd.run().await?,
        CastSubcommand::GasPrice { rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let out = Cast::new(provider).gas_price().await?.to_string();
            print_scalar(out)?;
        }
        CastSubcommand::Index { key_type, key, slot_number } => {
            let out = SimpleCast::index(&key_type, &key, &slot_number)?;
            print_scalar(out)?;
        }
        CastSubcommand::IndexErc7201 { id, formula_id } => {
            eyre::ensure!(formula_id == "erc7201", "unsupported formula ID: {formula_id}");
            let id = stdin::unwrap_line(id)?;
            let out = foundry_common::erc7201(&id).to_string();
            print_scalar(out)?;
        }
        CastSubcommand::Implementation { block, beacon, who, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            let out = Cast::new(provider).implementation(who, beacon, block).await?;
            print_scalar(out)?;
        }
        CastSubcommand::Admin { block, who, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            let out = Cast::new(provider).admin(who, block).await?;
            print_scalar(out)?;
        }
        CastSubcommand::Nonce { block, who, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            let out = Cast::new(provider).nonce(who, block).await?;
            print_scalar(out)?;
        }
        CastSubcommand::Codehash { block, who, slots, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            let out = Cast::new(provider).codehash(who, slots, block).await?;
            print_scalar(out)?;
        }
        CastSubcommand::StorageRoot { block, who, slots, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            let out = Cast::new(provider).storage_root(who, slots, block).await?;
            print_scalar(out)?;
        }
        CastSubcommand::ChannelId {
            payer,
            payee,
            token,
            salt,
            operator,
            authorized_signer,
            expiring_nonce_hash,
            reserve,
            block,
            rpc,
        } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let payer = payer.resolve(&provider).await?;
            let payee = payee.resolve(&provider).await?;
            let token = token.resolve(&provider).await?;
            let operator = match operator {
                Some(operator) => operator.resolve(&provider).await?,
                None => Address::ZERO,
            };
            let authorized_signer = match authorized_signer {
                Some(authorized_signer) => authorized_signer.resolve(&provider).await?,
                None => Address::ZERO,
            };
            let reserve = match reserve {
                Some(reserve) => reserve.resolve(&provider).await?,
                None => TIP20_CHANNEL_RESERVE_ADDRESS,
            };

            let channel_id = ITIP20ChannelReserve::new(reserve, &provider)
                .computeChannelId(
                    payer,
                    payee,
                    operator,
                    token,
                    salt,
                    authorized_signer,
                    expiring_nonce_hash,
                )
                .block(block.unwrap_or_default())
                .call()
                .await?;
            print_scalar(format!("{channel_id:#x}"))?;
        }
        CastSubcommand::Proof { address, slots, rpc, block } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let address = address.resolve(&provider).await?;
            let value = provider
                .get_proof(address, slots.into_iter().collect())
                .block_id(block.unwrap_or_default())
                .await?;
            print_json_object(value)?;
        }
        CastSubcommand::Rpc(cmd) => cmd.run().await?,
        CastSubcommand::Storage(cmd) => cmd.run().await?,

        // Calls & transactions
        CastSubcommand::Call(cmd) => cmd.run().await?,
        CastSubcommand::Estimate(cmd) => cmd.run().await?,
        CastSubcommand::MakeTx(cmd) => cmd.run().await?,
        CastSubcommand::PublishTx { raw_tx, cast_async, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            let cast = Cast::new(&provider);
            let pending_tx = cast.publish(raw_tx).await?;
            let tx_hash = pending_tx.inner().tx_hash();

            if cast_async {
                print_scalar(format!("{tx_hash:#x}"))?;
            } else {
                let receipt = pending_tx.get_receipt().await?;
                print_json_object(receipt)?;
            }
        }
        CastSubcommand::Receipt { tx_hash, field, cast_async, confirmations, rpc } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;
            // JSON: Output is already formatted by `Cast::format_receipt()`
            sh_println!(
                "{}",
                CastTxSender::new(provider)
                    .receipt(tx_hash, field, confirmations, None, cast_async)
                    .await?
            )?
        }
        CastSubcommand::Run(cmd) => cmd.run().await?,
        CastSubcommand::SendTx(cmd) => cmd.run().await?,
        CastSubcommand::BatchMakeTx(cmd) => cmd.run().await?,
        CastSubcommand::BatchSend(cmd) => cmd.run().await?,
        CastSubcommand::Classify { raw_tx } => {
            let raw_tx = stdin::unwrap_line(raw_tx)?;
            print_json_value_or_scalar(classify_raw_transaction_output(&raw_tx)?)?
        }
        CastSubcommand::Tx { tx_hash, from, nonce, field, raw, lane, rpc, to_request, network } => {
            let config = rpc.load_config()?;
            // Can use either --raw or specify raw as a field
            let is_raw = raw || field.as_ref().is_some_and(|f| f == "raw");
            let output = match network {
                #[cfg(feature = "optimism")]
                Some(NetworkVariant::Optimism) => {
                    let provider = ProviderBuilder::<Optimism>::from_config(&config)?.build()?;

                    Cast::new(&provider)
                        .transaction(tx_hash, from, nonce, field, is_raw, to_request, lane)
                        .await?
                }
                Some(NetworkVariant::Tempo) => {
                    let provider = tempo_provider(&config)?;
                    Cast::new(&provider)
                        .transaction(tx_hash, from, nonce, field, is_raw, to_request, lane)
                        .await?
                }
                // Ethereum (default) or no --raw flag
                _ => {
                    let provider = utils::get_provider(&config)?;
                    Cast::new(&provider)
                        .transaction(tx_hash, from, nonce, field, is_raw, to_request, lane)
                        .await?
                }
            };
            print_json_value_or_scalar(output)?;
        }

        // 4Byte
        CastSubcommand::FourByte { selector } => {
            let selector = stdin::unwrap_line(selector)?;
            let sigs = decode_function_selector(selector).await?;
            if sigs.is_empty() {
                eyre::bail!("No matching function signatures found for selector `{selector}`");
            }
            print_list(&sigs)?;
        }

        // JSON envelope intentionally unsupported: output combines an interactive selector
        // disambiguation step with decoded token output; no single stable shape exists.
        CastSubcommand::FourByteCalldata { calldata } => {
            let calldata = stdin::unwrap_line(calldata)?;

            if calldata.len() == 10 {
                let sigs = decode_function_selector(calldata.parse()?).await?;
                if sigs.is_empty() {
                    eyre::bail!("No matching function signatures found for calldata `{calldata}`");
                }
                for sig in sigs {
                    sh_println!("{sig}")?
                }
                return Ok(());
            }

            let sigs = decode_calldata(&calldata).await?;
            sigs.iter().enumerate().for_each(|(i, sig)| {
                let _ = sh_println!("{}) \"{sig}\"", i + 1);
            });

            let sig = match sigs.len() {
                0 => eyre::bail!("No signatures found"),
                1 => sigs.first().unwrap(),
                _ => {
                    let i: usize = prompt!("Select a function signature by number: ")?;
                    sigs.get(i - 1).ok_or_else(|| eyre::eyre!("Invalid signature index"))?
                }
            };

            let tokens = SimpleCast::calldata_decode(sig, &calldata, true)?;
            print_tokens(&tokens)?;
        }

        CastSubcommand::FourByteEvent { topic } => {
            let topic = stdin::unwrap_line(topic)?;
            let sigs = decode_event_topic(topic).await?;
            if sigs.is_empty() {
                eyre::bail!("No matching event signatures found for topic `{topic}`");
            }
            print_list(&sigs)?;
        }
        // JSON envelope intentionally unsupported: output is a human-readable summary from an
        // external selector registry API with no stable machine-readable schema.
        CastSubcommand::UploadSignature { signatures } => {
            let signatures = stdin::unwrap_vec(signatures)?;
            let ParsedSignatures { signatures, abis } = parse_signatures(signatures);
            if !abis.is_empty() {
                import_selectors(SelectorImportData::Abi(abis)).await?.describe();
            }
            if !signatures.is_empty() {
                import_selectors(SelectorImportData::Raw(signatures)).await?.describe();
            }
        }

        // ENS
        CastSubcommand::Namehash { name } => {
            let name = stdin::unwrap_line(name)?;
            let out = namehash(&name).to_string();
            print_scalar(out)?;
        }
        CastSubcommand::LookupAddress { who, rpc, verify } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;

            let who = stdin::unwrap_line(who)?;
            let name = provider.lookup_address(&who).await?;
            if verify {
                let address = provider.resolve_name(&name).await?;
                eyre::ensure!(
                    address == who,
                    "Reverse lookup verification failed: got `{address}`, expected `{who}`"
                );
            }
            print_scalar(name)?;
        }
        CastSubcommand::ResolveName { who, rpc, verify } => {
            let config = rpc.load_config()?;
            let provider = utils::get_provider(&config)?;

            let who = stdin::unwrap_line(who)?;
            let address = provider
                .resolve_name(&who)
                .await
                .wrap_err(format!("Failed to resolve ENS name: {who}"))?;
            if verify {
                let name = provider.lookup_address(&address).await?;
                eyre::ensure!(
                    name == who,
                    "Forward lookup verification failed: got `{name}`, expected `{who}`"
                );
            }
            print_scalar(address.to_string())?;
        }

        // Misc
        CastSubcommand::Keccak { data } => {
            let bytes = match data {
                Some(data) => data.into_bytes(),
                None => stdin::read_bytes(false)?,
            };
            let out = match String::from_utf8(bytes) {
                Ok(s) => SimpleCast::keccak(&s)?,
                Err(e) => format!("0x{}", hex::encode(keccak256(e.as_bytes()))),
            };
            print_scalar(out)?;
        }
        CastSubcommand::HashMessage { message } => {
            let message = stdin::unwrap(message, false)?;
            let out = eip191_hash_message(message).to_string();
            print_scalar(out)?;
        }
        CastSubcommand::SigEvent { event_string } => {
            let event_string = stdin::unwrap_line(event_string)?;
            let parsed_event = get_event(&event_string)?;
            print_scalar(format!("{:?}", parsed_event.selector()))?;
        }
        CastSubcommand::LeftShift { value, bits, base_in, base_out } => {
            let out = SimpleCast::left_shift(&value, &bits, base_in.as_deref(), &base_out)?;
            print_scalar(out)?;
        }
        CastSubcommand::RightShift { value, bits, base_in, base_out } => {
            let out = SimpleCast::right_shift(&value, &bits, base_in.as_deref(), &base_out)?;
            print_scalar(out)?;
        }
        // TODO(json): multi-line source code or directory expansion, needs structured envelope
        CastSubcommand::Source {
            address,
            directory,
            explorer_api_url,
            explorer_url,
            etherscan,
            flatten,
        } => {
            let config = etherscan.load_config()?;
            let chain = config.chain.unwrap_or_default();
            let api_key = config.get_etherscan_api_key(Some(chain));
            match (directory, flatten) {
                (Some(dir), false) => {
                    SimpleCast::expand_etherscan_source_to_directory(
                        chain,
                        address,
                        api_key,
                        dir,
                        explorer_api_url,
                        explorer_url,
                    )
                    .await?
                }
                (None, false) => sh_println!(
                    "{}",
                    SimpleCast::etherscan_source(
                        chain,
                        address,
                        api_key,
                        explorer_api_url,
                        explorer_url
                    )
                    .await?
                )?,
                (dir, true) => {
                    SimpleCast::etherscan_source_flatten(
                        chain,
                        address,
                        api_key,
                        dir,
                        explorer_api_url,
                        explorer_url,
                    )
                    .await?;
                }
            }
        }
        CastSubcommand::Create2(cmd) => {
            cmd.run()?;
        }
        CastSubcommand::Wallet { command } => command.run().await?,
        CastSubcommand::Completions { shell } => {
            generate(shell, &mut CastArgs::command(), "cast", &mut std::io::stdout())
        }
        CastSubcommand::Logs(cmd) => cmd.run().await?,
        CastSubcommand::DecodeTransaction { tx, network } => {
            let tx = stdin::unwrap_line(tx)?;
            let decoded_tx = match network {
                #[cfg(feature = "optimism")]
                Some(NetworkVariant::Optimism) => {
                    SimpleCast::decode_raw_transaction::<Optimism>(&tx)?
                }
                Some(NetworkVariant::Tempo) => {
                    SimpleCast::decode_raw_transaction::<TempoNetwork>(&tx)?
                }
                Some(NetworkVariant::Ethereum) => {
                    SimpleCast::decode_raw_transaction::<Ethereum>(&tx)?
                }
                // Without an explicit `--network` override, decode with the Foundry envelope,
                // which dispatches on the EIP-2718 type byte for the transaction types compiled
                // into `FoundryNetwork`, including Tempo txs (`0x76`).
                None => SimpleCast::decode_raw_transaction::<FoundryNetwork>(&tx)?,
            };
            print_json_object(decoded_tx)?;
        }
        CastSubcommand::RecoverAuthority { auth } => {
            let auth: SignedAuthorization = serde_json::from_str(&auth)?;
            let out = auth.recover_authority()?.to_string();
            print_scalar(out)?;
        }
        CastSubcommand::TxPool { command } => command.run().await?,
        CastSubcommand::Erc20Token { command } => command.run().await?,
        CastSubcommand::Tip20Token { command } => command.run().await?,
        CastSubcommand::ReceivePolicy { command } => command.run().await?,
        CastSubcommand::Tip403 { command } => command.run().await?,
        CastSubcommand::StorageCredits { command } => command.run().await?,
        CastSubcommand::Keychain { command } => command.run().await?,
        CastSubcommand::KeyAuthorization { command } => command.run().await?,
        CastSubcommand::Tempo { command } => command.run().await?,
        CastSubcommand::VirtualAddress { command } => command.run().await?,
        #[cfg(feature = "optimism")]
        CastSubcommand::DAEstimate(cmd) => {
            cmd.run().await?;
        }
        CastSubcommand::Trace(cmd) => cmd.run().await?,
    };

    Ok(())
}

pub(crate) fn classify_raw_transaction_output(raw_tx: &str) -> Result<String> {
    let raw_tx = hex::decode(raw_tx)?;
    let mut data = raw_tx.as_slice();
    FoundryTxEnvelope::decode_2718(&mut data).wrap_err("failed to decode raw transaction")?;
    format_lane_classification(&classify_payment_lane(&raw_tx))
}

pub(crate) fn format_lane_classification(
    classification: &PaymentLaneClassification,
) -> Result<String> {
    if shell::is_json() {
        Ok(serde_json::to_string_pretty(classification)?)
    } else {
        Ok(serde_json::to_string(classification)?)
    }
}
