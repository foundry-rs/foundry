use crate::{
    Cast, SimpleCast,
    cmd::{erc20::IERC20, rpc_provider},
    opts::{Cast as CastArgs, CastSubcommand, ToBaseArgs},
    traces::identifier::SignaturesIdentifier,
    tx::CastTxSender,
};
use alloy_consensus::Typed2718;
use alloy_dyn_abi::{ErrorExt, EventExt};
use alloy_eips::{Encodable2718, eip7702::SignedAuthorization};
use alloy_ens::{NameOrAddress, ProviderEnsExt, namehash};
use alloy_network::{Ethereum, eip2718::Decodable2718};
use alloy_primitives::{Address, B256, Bytes, eip191_hash_message, hex, keccak256};
use alloy_provider::Provider;
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use eyre::{Result, WrapErr};
use foundry_cli::{
    json::{print_json_object, print_json_value_or_scalar, print_list, print_scalar, print_tokens},
    opts::RpcOpts,
    utils::{self, LoadConfig},
};
use foundry_common::{
    abi::{get_error, get_event},
    fmt::format_uint_exp,
    fs,
    provider::{ProviderBuilder, RetryProvider},
    selectors::{
        ParsedSignatures, SelectorImportData, SelectorKind, decode_calldata, decode_event_topic,
        decode_function_selector, decode_selectors, import_selectors, parse_signatures,
        pretty_calldata,
    },
    shell, stdin,
    tempo::classify_payment_lane,
};
use foundry_evm_networks::NetworkVariant;
use foundry_primitives::{FoundryNetwork, FoundryTxEnvelope};
#[cfg(feature = "optimism")]
use op_alloy_network::Optimism;
use std::time::Instant;
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{ITIP20ChannelReserve, TIP20_CHANNEL_RESERVE_ADDRESS};

/// Runs `$body` with `$provider` bound to a provider for the selected `--network`.
///
/// The fallback arm is used for Ethereum and when no network is selected: either `$default` is a
/// provider expression that `$body` runs against, or a full `_ => $default` arm.
macro_rules! with_network_provider {
    ($network:expr, $config:expr, $default:expr, |$provider:ident| $body:expr) => {
        with_network_provider!($network, $config, |$provider| $body, _ => {
            let $provider = $default;
            $body
        })
    };
    ($network:expr, $config:expr, |$provider:ident| $body:expr, _ => $default:expr) => {
        match $network {
            #[cfg(feature = "optimism")]
            Some(NetworkVariant::Optimism) => {
                let $provider = ProviderBuilder::<Optimism>::from_config($config)?.build()?;
                $body
            }
            Some(NetworkVariant::Tempo) => {
                let $provider = ProviderBuilder::<TempoNetwork>::from_config($config)?.build()?;
                $body
            }
            _ => $default,
        }
    };
}

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
        CastSubcommand::MaxInt { r#type } | CastSubcommand::MaxUint { r#type } => {
            print_scalar(SimpleCast::max_int(&r#type)?)?;
        }
        CastSubcommand::MinInt { r#type } => print_scalar(SimpleCast::min_int(&r#type)?)?,
        CastSubcommand::AddressZero => print_scalar(format!("{:?}", Address::ZERO))?,
        CastSubcommand::HashZero => print_scalar(format!("{:?}", B256::ZERO))?,

        // Conversions & transformations
        CastSubcommand::FromUtf8 { text } => {
            print_scalar(SimpleCast::from_utf8(&stdin::unwrap(text, false)?))?;
        }
        CastSubcommand::ToAscii { hexdata } => {
            print_scalar(SimpleCast::to_ascii(stdin::unwrap(hexdata, false)?.trim())?)?;
        }
        CastSubcommand::ToUtf8 { hexdata } => {
            print_scalar(SimpleCast::to_utf8(&stdin::unwrap(hexdata, false)?)?)?;
        }
        CastSubcommand::FromFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;
            print_scalar(SimpleCast::from_fixed_point(&value, &decimals)?)?;
        }
        CastSubcommand::ToFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;
            print_scalar(SimpleCast::to_fixed_point(&value, &decimals)?)?;
        }
        CastSubcommand::ConcatHex { data } => {
            let out = if data.is_empty() {
                SimpleCast::concat_hex(stdin::read(true)?.split_whitespace())
            } else {
                SimpleCast::concat_hex(data)
            };
            print_scalar(out)?;
        }
        CastSubcommand::FromBin => {
            print_scalar(hex::encode_prefixed(stdin::read_bytes(false)?))?;
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
            print_scalar(stdin::unwrap_line(address)?.to_checksum(chain_id))?;
        }
        CastSubcommand::ToUint256 { value } => {
            print_scalar(SimpleCast::to_uint256(&stdin::unwrap_line(value)?)?)?;
        }
        CastSubcommand::ToInt256 { value } => {
            print_scalar(SimpleCast::to_int256(&stdin::unwrap_line(value)?)?)?;
        }
        CastSubcommand::ToUnit { value, unit } => {
            print_scalar(SimpleCast::to_unit(&stdin::unwrap_line(value)?, &unit)?)?;
        }
        CastSubcommand::ParseUnits { value, unit } => {
            print_scalar(SimpleCast::parse_units(&stdin::unwrap_line(value)?, unit)?)?;
        }
        CastSubcommand::FormatUnits { value, unit } => {
            print_scalar(SimpleCast::format_units(&stdin::unwrap_line(value)?, unit)?)?;
        }
        CastSubcommand::FromWei { value, unit } => {
            print_scalar(SimpleCast::from_wei(&stdin::unwrap_line(value)?, &unit)?)?;
        }
        CastSubcommand::ToWei { value, unit } => {
            print_scalar(SimpleCast::to_wei(&stdin::unwrap_line(value)?, &unit)?)?;
        }
        CastSubcommand::FromRlp { value, as_int } => {
            print_scalar(SimpleCast::from_rlp(stdin::unwrap_line(value)?, as_int)?)?;
        }
        CastSubcommand::ToRlp { value } => {
            print_scalar(SimpleCast::to_rlp(&stdin::unwrap_line(value)?)?)?;
        }
        CastSubcommand::ToHex(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            print_scalar(SimpleCast::to_base(&value, base_in.as_deref(), "hex")?)?;
        }
        CastSubcommand::ToDec(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            print_scalar(SimpleCast::to_base(&value, base_in.as_deref(), "dec")?)?;
        }
        CastSubcommand::ToBase { base: ToBaseArgs { value, base_in }, base_out } => {
            let (value, base_out) = stdin::unwrap2(value, base_out)?;
            print_scalar(SimpleCast::to_base(&value, base_in.as_deref(), &base_out)?)?;
        }
        CastSubcommand::ToBytes32 { bytes } => {
            print_scalar(SimpleCast::to_bytes32(&stdin::unwrap_line(bytes)?)?)?;
        }
        CastSubcommand::ToBytesMemory { data } => {
            print_scalar(SimpleCast::to_bytes_memory(&stdin::unwrap_line(data)?)?)?;
        }
        CastSubcommand::Pad { data, right, left: _, len } => {
            print_scalar(SimpleCast::pad(&stdin::unwrap_line(data)?, right, len)?)?;
        }
        CastSubcommand::FormatBytes32String { string } => {
            print_scalar(SimpleCast::format_bytes32_string(&stdin::unwrap_line(string)?)?)?;
        }
        CastSubcommand::ParseBytes32String { bytes } => {
            print_scalar(SimpleCast::parse_bytes32_string(&stdin::unwrap_line(bytes)?)?)?;
        }
        CastSubcommand::ParseBytes32Address { bytes } => {
            print_scalar(SimpleCast::parse_bytes32_address(&stdin::unwrap_line(bytes)?)?)?;
        }

        // ABI encoding & decoding
        CastSubcommand::DecodeAbi { sig, calldata, input } => {
            print_tokens(&SimpleCast::abi_decode(&sig, &calldata, input)?)?;
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
                print_json_object(EncodedEvent {
                    topics: log_data.topics().iter().map(|t| t.to_string()).collect(),
                    data: hex::encode_prefixed(&log_data.data),
                })?;
            } else {
                for (i, topic) in log_data.topics().iter().enumerate() {
                    sh_println!("[topic{i}]: {topic}")?;
                }
                if !log_data.data.is_empty() {
                    sh_println!("[data]: {}", hex::encode_prefixed(log_data.data))?;
                }
            }
        }
        CastSubcommand::DecodeCalldata { sig, calldata, file } => {
            let raw_hex = match file {
                Some(file_path) => fs::read_to_string(&file_path)?.trim().to_string(),
                None => calldata.unwrap(),
            };
            print_tokens(&SimpleCast::calldata_decode(&sig, &raw_hex, true)?)?;
        }
        CastSubcommand::CalldataEncode { sig, args, file } => {
            let args = match file {
                Some(file_path) => fs::read_to_string(file_path)?
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(String::from)
                    .collect(),
                None => args,
            };
            print_scalar(SimpleCast::calldata_encode(sig, &args)?)?;
        }
        CastSubcommand::DecodeString { data } => {
            print_tokens(&SimpleCast::calldata_decode("Any(string)", &data, true)?)?;
        }
        CastSubcommand::DecodeEvent { sig, data } => {
            let decoded_event = if let Some(event_sig) = sig {
                let event = get_event(&event_sig)?;
                event.decode_log_parts(core::iter::once(event.selector()), &hex::decode(data)?)?
            } else {
                let data = crate::strip_0x(&data);
                let selector: B256 = data.get(..64).unwrap_or_default().parse()?;
                let Some(event) = SignaturesIdentifier::new(false)?.identify_event(selector).await
                else {
                    eyre::bail!("No matching event signature found for selector `{selector}`");
                };
                let _ = sh_println!("{}", event.signature());
                let data = data.get(64..).unwrap_or_default();
                get_event(&event.signature())?
                    .decode_log_parts(core::iter::once(selector), &hex::decode(data)?)?
            };
            print_tokens(&decoded_event.body)?;
        }
        CastSubcommand::DecodeError { sig, data } => {
            let error = if let Some(err_sig) = sig {
                get_error(&err_sig)?
            } else {
                let data = crate::strip_0x(&data);
                let selector = data.get(..8).unwrap_or_default();
                let Some(error) =
                    SignaturesIdentifier::new(false)?.identify_error(selector.parse()?).await
                else {
                    eyre::bail!("No matching error signature found for selector `{selector}`");
                };
                let _ = sh_println!("{}", error.signature());
                error
            };
            print_tokens(&error.decode_error(&hex::decode(data)?)?.body)?;
        }
        CastSubcommand::Interface(cmd) => cmd.run().await?,
        CastSubcommand::CreationCode(cmd) => cmd.run().await?,
        CastSubcommand::ConstructorArgs(cmd) => cmd.run().await?,
        CastSubcommand::Artifact(cmd) => cmd.run().await?,
        CastSubcommand::Bind(cmd) => cmd.run().await?,
        CastSubcommand::B2EPayload(cmd) => cmd.run().await?,
        CastSubcommand::PrettyCalldata { calldata, offline } => {
            let calldata = stdin::unwrap_line(calldata)?;
            print_scalar(pretty_calldata(&calldata, offline).await?.to_string())?;
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
                None => print_scalar(SimpleCast::get_selector(&sig, 0)?.0)?,
            }
        }

        // Blockchain & RPC queries
        CastSubcommand::AccessList(cmd) => cmd.run().await?,
        CastSubcommand::Age { block, rpc } => {
            let age = Cast::new(rpc_provider(&rpc)?).age(block.unwrap_or_default()).await?;
            print_scalar(format!("{age} UTC"))?;
        }
        CastSubcommand::Balance { block, who, ether, rpc, erc20, overrides } => {
            if erc20.is_none() && !overrides.is_empty() {
                eyre::bail!("call overrides require `--erc20` when using `cast balance`");
            }
            let (provider, account_addr) = rpc_provider_and_address(&rpc, who).await?;

            match erc20 {
                Some(token) => {
                    let token = IERC20::new(token, &provider);
                    let balance_call =
                        token.balanceOf(account_addr).block(block.unwrap_or_default());
                    let balance = overrides.apply(balance_call.call())?.await?;

                    sh_warn!("--erc20 flag is deprecated, use `cast erc20 balance` instead")?;
                    print_scalar(format_uint_exp(balance))?;
                }
                None => {
                    let value = provider
                        .get_balance(account_addr)
                        .block_id(block.unwrap_or_default())
                        .await?;
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
            let fee = Cast::new(rpc_provider(&rpc)?).base_fee(block.unwrap_or_default()).await?;
            print_scalar(fee.to_string())?;
        }
        CastSubcommand::Block { block, full, fields, raw, rpc, network } => {
            let config = rpc.load_config()?;
            let block = block.unwrap_or_default();
            // Can use either --raw or specify raw as a field
            let output = if raw || fields.contains(&"raw".into()) {
                with_network_provider!(
                    network,
                    &config,
                    ProviderBuilder::<Ethereum>::from_config(&config)?.build()?,
                    |provider| Cast::new(&provider).block_raw(block, full).await?
                )
            } else {
                Cast::new(utils::get_provider(&config)?).block(block, full, fields).await?
            };
            print_json_value_or_scalar(output)?;
        }
        CastSubcommand::BlockNumber { rpc, block } => {
            let provider = rpc_provider(&rpc)?;
            let number = match block {
                Some(id) => {
                    provider
                        .get_block(id)
                        .await?
                        .ok_or_else(|| eyre::eyre!("block {id:?} not found"))?
                        .header
                        .number
                }
                None => provider.get_block_number().await?,
            };
            print_scalar(number)?;
        }
        CastSubcommand::Chain { rpc } => {
            print_scalar(Cast::new(rpc_provider(&rpc)?).chain().await?.to_string())?;
        }
        CastSubcommand::ChainId { rpc } => {
            print_scalar(rpc_provider(&rpc)?.get_chain_id().await?.to_string())?;
        }
        CastSubcommand::Client { rpc } => {
            print_scalar(rpc_provider(&rpc)?.get_client_version().await?)?;
        }
        CastSubcommand::Code { block, who, disassemble, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(Cast::new(provider).code(who, block, disassemble).await?)?;
        }
        CastSubcommand::Codesize { block, who, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(Cast::new(provider).codesize(who, block).await?)?;
        }
        CastSubcommand::ComputeAddress { address, nonce, salt, init_code, init_code_hash, rpc } => {
            let address = stdin::unwrap_line(address)?;
            let salt = salt.unwrap_or(B256::ZERO);
            let computed = if let Some(init_code_hash) = init_code_hash {
                address.create2(salt, init_code_hash)
            } else if let Some(init_code) = init_code {
                address.create2(salt, keccak256(hex::decode(init_code)?))
            } else {
                // CREATE addresses depend on the deployer nonce, which is fetched over RPC.
                Cast::new(rpc_provider(&rpc)?).compute_address(address, nonce).await?
            };
            print_scalar(computed.to_checksum(None))?;
        }
        CastSubcommand::Disassemble { bytecode } => {
            let bytecode = stdin::unwrap_line(bytecode)?;
            print_scalar(SimpleCast::disassemble(&hex::decode(bytecode)?)?)?;
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
                let infos = functions
                    .into_iter()
                    .enumerate()
                    .map(|(pos, (selector, arguments, state_mutability))| SelectorInfo {
                        selector: selector.to_string(),
                        arguments,
                        state_mutability: state_mutability.to_string(),
                        resolved: resolve_results.get(pos).cloned(),
                    })
                    .collect::<Vec<_>>();
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
            print_scalar(Cast::new(rpc_provider(&rpc)?).gas_price().await?.to_string())?;
        }
        CastSubcommand::Index { key_type, key, slot_number } => {
            print_scalar(SimpleCast::index(&key_type, &key, &slot_number)?)?;
        }
        CastSubcommand::IndexErc7201 { id, formula_id } => {
            eyre::ensure!(formula_id == "erc7201", "unsupported formula ID: {formula_id}");
            let id = stdin::unwrap_line(id)?;
            print_scalar(foundry_common::erc7201(&id).to_string())?;
        }
        CastSubcommand::Implementation { block, beacon, who, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(Cast::new(provider).implementation(who, beacon, block).await?)?;
        }
        CastSubcommand::Admin { block, who, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(Cast::new(provider).admin(who, block).await?)?;
        }
        CastSubcommand::Nonce { block, who, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(Cast::new(provider).nonce(who, block).await?)?;
        }
        CastSubcommand::Codehash { block, who, slots, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(Cast::new(provider).codehash(who, slots, block).await?)?;
        }
        CastSubcommand::StorageRoot { block, who, slots, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(Cast::new(provider).storage_root(who, slots, block).await?)?;
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
            let provider = rpc_provider(&rpc)?;
            let payer = payer.resolve(&provider).await?;
            let payee = payee.resolve(&provider).await?;
            let token = token.resolve(&provider).await?;
            let operator = resolve_or(operator, Address::ZERO, &provider).await?;
            let authorized_signer = resolve_or(authorized_signer, Address::ZERO, &provider).await?;
            let reserve = resolve_or(reserve, TIP20_CHANNEL_RESERVE_ADDRESS, &provider).await?;

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
            let (provider, address) = rpc_provider_and_address(&rpc, address).await?;
            let value =
                provider.get_proof(address, slots).block_id(block.unwrap_or_default()).await?;
            print_json_object(value)?;
        }
        CastSubcommand::Rpc(cmd) => cmd.run().await?,
        CastSubcommand::Storage(cmd) => cmd.run().await?,

        // Calls & transactions
        CastSubcommand::Call(cmd) => cmd.run().await?,
        CastSubcommand::Estimate(cmd) => cmd.run().await?,
        CastSubcommand::MakeTx(cmd) => cmd.run().await?,
        CastSubcommand::PublishTx { raw_tx, cast_async, rpc } => {
            let provider = rpc_provider(&rpc)?;
            let pending_tx = Cast::new(&provider).publish(raw_tx).await?;
            if cast_async {
                print_scalar(format!("{:#x}", pending_tx.inner().tx_hash()))?;
            } else {
                print_json_object(pending_tx.get_receipt().await?)?;
            }
        }
        CastSubcommand::Receipt { tx_hash, field, cast_async, confirmations, rpc } => {
            // JSON: Output is already formatted by `Cast::format_receipt()`
            sh_println!(
                "{}",
                CastTxSender::new(rpc_provider(&rpc)?)
                    .receipt(tx_hash, field, confirmations, None, cast_async)
                    .await?
            )?
        }
        CastSubcommand::Run(cmd) => cmd.run().await?,
        CastSubcommand::SendTx(cmd) => cmd.run().await?,
        CastSubcommand::BatchMakeTx(cmd) => cmd.run().await?,
        CastSubcommand::BatchSend(cmd) => cmd.run().await?,
        CastSubcommand::Classify { raw_tx } => {
            let raw_tx = hex::decode(stdin::unwrap_line(raw_tx)?)?;
            let out = format_lane_classification(&raw_tx, "failed to decode raw transaction")?;
            print_json_value_or_scalar(out)?
        }
        CastSubcommand::Tx { tx_hash, from, nonce, field, raw, lane, rpc, to_request, network } => {
            let config = rpc.load_config()?;
            // Can use either --raw or specify raw as a field
            let is_raw = raw || field.as_deref() == Some("raw");
            let output = if is_raw || lane {
                let encoded: Bytes = with_network_provider!(
                    network,
                    &config,
                    |provider| {
                        let tx =
                            Cast::new(&provider).transaction_response(tx_hash, from, nonce).await?;
                        tx.as_ref().encoded_2718().into()
                    },
                    _ => {
                        let provider = utils::get_provider(&config)?;
                        let tx =
                            Cast::new(&provider).transaction_response(tx_hash, from, nonce).await?;
                        FoundryTxEnvelope::encode_rpc_2718(&tx).wrap_err_with(|| {
                            format!("Cannot EIP-2718 encode transaction type 0x{:x}", tx.ty())
                        })?
                    }
                );
                if lane {
                    format_lane_classification(
                        &encoded,
                        "failed to decode transaction for lane classification",
                    )?
                } else {
                    hex::encode_prefixed(encoded)
                }
            } else {
                with_network_provider!(
                    network,
                    &config,
                    utils::get_provider(&config)?,
                    |provider| {
                        Cast::new(&provider)
                            .transaction(tx_hash, from, nonce, field, false, to_request)
                            .await?
                    }
                )
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
            for (i, sig) in sigs.iter().enumerate() {
                let _ = sh_println!("{}) \"{sig}\"", i + 1);
            }

            let sig = match sigs.len() {
                0 => eyre::bail!("No signatures found"),
                1 => &sigs[0],
                _ => {
                    let i: usize = prompt!("Select a function signature by number: ")?;
                    sigs.get(i - 1).ok_or_else(|| eyre::eyre!("Invalid signature index"))?
                }
            };

            print_tokens(&SimpleCast::calldata_decode(sig, &calldata, true)?)?;
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
            print_scalar(namehash(&stdin::unwrap_line(name)?).to_string())?;
        }
        CastSubcommand::LookupAddress { who, rpc, verify } => {
            let provider = rpc_provider(&rpc)?;
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
            let provider = rpc_provider(&rpc)?;
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
                Err(e) => hex::encode_prefixed(keccak256(e.as_bytes())),
            };
            print_scalar(out)?;
        }
        CastSubcommand::HashMessage { message } => {
            print_scalar(eip191_hash_message(stdin::unwrap(message, false)?).to_string())?;
        }
        CastSubcommand::SigEvent { event_string } => {
            let event = get_event(&stdin::unwrap_line(event_string)?)?;
            print_scalar(format!("{:?}", event.selector()))?;
        }
        CastSubcommand::LeftShift { value, bits, base_in, base_out } => {
            print_scalar(SimpleCast::left_shift(&value, &bits, base_in.as_deref(), &base_out)?)?;
        }
        CastSubcommand::RightShift { value, bits, base_in, base_out } => {
            print_scalar(SimpleCast::right_shift(&value, &bits, base_in.as_deref(), &base_out)?)?;
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
        CastSubcommand::Create2(cmd) => cmd.execute()?,
        CastSubcommand::Wallet { command } => command.run().await?,
        CastSubcommand::Safe { command } => command.run().await?,
        CastSubcommand::Completions { shell } => {
            generate(shell, &mut CastArgs::command(), "cast", &mut std::io::stdout())
        }
        CastSubcommand::Logs(cmd) => cmd.run().await?,
        CastSubcommand::Events(cmd) => cmd.run().await?,
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
                #[cfg(feature = "monad")]
                Some(NetworkVariant::Monad) => SimpleCast::decode_raw_transaction::<Ethereum>(&tx)?,
                // Without an explicit `--network` override, decode with the Foundry envelope,
                // which dispatches on the EIP-2718 type byte for the transaction types compiled
                // into `FoundryNetwork`, including Tempo txs (`0x76`).
                None => SimpleCast::decode_raw_transaction::<FoundryNetwork>(&tx)?,
            };
            print_json_object(decoded_tx)?;
        }
        CastSubcommand::RecoverAuthority { auth } => {
            let auth: SignedAuthorization = serde_json::from_str(&auth)?;
            print_scalar(auth.recover_authority()?.to_string())?;
        }
        CastSubcommand::TxPool { command } => command.run().await?,
        CastSubcommand::Erc20Token { command } => command.run().await?,
        CastSubcommand::Erc4626 { command } => command.run().await?,
        CastSubcommand::Tip20Token { command } => command.run().await?,
        CastSubcommand::ReceivePolicy { command } => command.run().await?,
        CastSubcommand::Tip403 { command } => command.run().await?,
        CastSubcommand::StorageCredits { command } => command.run().await?,
        CastSubcommand::Keychain { command } => command.run().await?,
        CastSubcommand::KeyAuthorization { command } => command.run().await?,
        CastSubcommand::Tempo { command } => command.run().await?,
        CastSubcommand::VirtualAddress { command } => command.run().await?,
        #[cfg(feature = "optimism")]
        CastSubcommand::DAEstimate(cmd) => cmd.run().await?,
        CastSubcommand::Trace(cmd) => cmd.run().await?,
    };

    Ok(())
}

/// Builds the default provider for `rpc` and resolves `who` against it.
async fn rpc_provider_and_address(
    rpc: &RpcOpts,
    who: NameOrAddress,
) -> Result<(RetryProvider, Address)> {
    let provider = rpc_provider(rpc)?;
    let who = who.resolve(&provider).await?;
    Ok((provider, who))
}

/// Resolves `who` when given, otherwise returns `default`.
async fn resolve_or(
    who: Option<NameOrAddress>,
    default: Address,
    provider: &RetryProvider,
) -> Result<Address> {
    Ok(match who {
        Some(who) => who.resolve(provider).await?,
        None => default,
    })
}

/// Validates that `encoded` is a supported EIP-2718 transaction and renders its Tempo T5 payment
/// lane classification.
fn format_lane_classification(encoded: &[u8], decode_context: &'static str) -> Result<String> {
    FoundryTxEnvelope::decode_2718(&mut &encoded[..]).wrap_err(decode_context)?;
    let classification = classify_payment_lane(encoded);
    if shell::is_json() {
        Ok(serde_json::to_string_pretty(&classification)?)
    } else {
        Ok(serde_json::to_string(&classification)?)
    }
}
