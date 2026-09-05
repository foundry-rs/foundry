use crate::{
    base::{Base, NumberWithBase},
    cmd::{erc20::IERC20, rpc_provider},
    opts::{Cast as CastArgs, CastSubcommand, ToBaseArgs},
    traces::identifier::SignaturesIdentifier,
    tx::CastTxSender,
};
use alloy_consensus::{
    Typed2718,
    transaction::{Recovered, SignerRecoverable},
};
use alloy_dyn_abi::{DynSolType, DynSolValue, ErrorExt, EventExt, Specifier};
use alloy_eips::{Encodable2718, eip7702::SignedAuthorization};
use alloy_ens::{NameOrAddress, ProviderEnsExt, namehash};
use alloy_network::{BlockResponse, Ethereum, Network, eip2718::Decodable2718};
use alloy_primitives::{
    Address, B256, Bytes, I256, Keccak256, LogData, TxHash, U64, U256, b256, eip191_hash_message,
    hex, keccak256,
    utils::{ParseUnits, Unit},
};
use alloy_provider::Provider;
use alloy_rlp::Decodable;
use alloy_rpc_types::BlockId;
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use eyre::{ContextCompat, OptionExt, Result, WrapErr};
use foundry_block_explorers::Client;
use foundry_cli::{
    json::{print_json_object, print_json_value_or_scalar, print_list, print_scalar, print_tokens},
    opts::RpcOpts,
    utils::{self, LoadConfig},
};
use foundry_common::{
    abi::{
        abi_decode_calldata, encode_function_args, encode_function_args_packed, get_error,
        get_event, get_func,
    },
    fmt::{UIfmt, UIfmtSignatureExt, format_uint_exp, get_pretty_block_attr, get_pretty_tx_attr},
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
use foundry_config::Chain;
use foundry_evm_networks::NetworkVariant;
use foundry_primitives::{FoundryNetwork, FoundryTxEnvelope};
#[cfg(feature = "optimism")]
use op_alloy_network::Optimism;
use rayon::prelude::*;
use serde::Serialize;
use std::{
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
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
            print_scalar(int_bound(&r#type, true)?)?;
        }
        CastSubcommand::MinInt { r#type } => print_scalar(int_bound(&r#type, false)?)?,
        CastSubcommand::AddressZero => print_scalar(format!("{:?}", Address::ZERO))?,
        CastSubcommand::HashZero => print_scalar(format!("{:?}", B256::ZERO))?,

        // Conversions & transformations
        CastSubcommand::FromUtf8 { text } => {
            print_scalar(hex::encode_prefixed(stdin::unwrap(text, false)?))?;
        }
        CastSubcommand::ToAscii { hexdata } => {
            let bytes = hex::decode(stdin::unwrap(hexdata, false)?.trim())?;
            eyre::ensure!(bytes.iter().all(u8::is_ascii), "Invalid ASCII bytes");
            print_scalar(String::from_utf8(bytes).unwrap())?;
        }
        CastSubcommand::ToUtf8 { hexdata } => {
            let bytes = hex::decode(stdin::unwrap(hexdata, false)?)?;
            print_scalar(String::from_utf8_lossy(&bytes).into_owned())?;
        }
        CastSubcommand::FromFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;
            print_scalar(ParseUnits::parse_units(&value, Unit::from_str(&decimals)?)?.to_string())?;
        }
        CastSubcommand::ToFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;

            let number = NumberWithBase::parse_int(&value, None)?;
            let sign = if number.is_nonnegative() { "" } else { "-" };
            let mut value = number.to_string().trim_start_matches('-').to_string();
            let value_len = value.len();
            let decimals_num = NumberWithBase::parse_uint(&decimals, None)?.number();
            let decimals: usize = decimals_num
                .try_into()
                .ok()
                .filter(|&d: &usize| d <= u16::MAX as usize)
                .ok_or_else(|| eyre::eyre!("decimals out of range: {decimals_num}"))?;

            if decimals >= value_len {
                value = format!("0.{value:0>decimals$}");
            } else {
                value.insert(value_len - decimals, '.');
            }
            print_scalar(format!("{sign}{value}"))?;
        }
        CastSubcommand::ConcatHex { data } => {
            let input;
            let values = if data.is_empty() {
                input = stdin::read(true)?;
                itertools::Either::Left(input.split_whitespace())
            } else {
                itertools::Either::Right(data.iter().map(String::as_str))
            };
            let out = values.map(strip_0x).collect::<String>();
            print_scalar(format!("0x{out}"))?;
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
            let n = NumberWithBase::parse_uint(&stdin::unwrap_line(value)?, None)?;
            print_scalar(format!("{n:#066x}"))?;
        }
        CastSubcommand::ToInt256 { value } => {
            let n = NumberWithBase::parse_int(&stdin::unwrap_line(value)?, None)?;
            print_scalar(format!("{n:#066x}"))?;
        }
        CastSubcommand::ToUnit { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            let value = DynSolType::coerce_str(&DynSolType::Uint(256), &value)?
                .as_uint()
                .wrap_err("Could not convert to uint")?
                .0;
            let unit = unit.parse().wrap_err("could not parse units")?;
            print_scalar(format_unit_as_string(ParseUnits::U256(value), unit))?;
        }
        CastSubcommand::ParseUnits { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            let unit = Unit::new(unit).ok_or_else(|| eyre::eyre!("invalid unit"))?;

            print_scalar(ParseUnits::parse_units(&value, unit)?.to_string())?;
        }
        CastSubcommand::FormatUnits { value, unit } => {
            print_scalar(format_units(&stdin::unwrap_line(value)?, unit)?)?;
        }
        CastSubcommand::FromWei { value, unit } => {
            print_scalar(
                signed_parse_units(&NumberWithBase::parse_int(&stdin::unwrap_line(value)?, None)?)?
                    .format_units(unit.parse()?),
            )?;
        }
        CastSubcommand::ToWei { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            let unit = unit.parse().wrap_err("could not parse units")?;
            print_scalar(ParseUnits::parse_units(&value, unit)?.to_string())?;
        }
        CastSubcommand::FromRlp { value, as_int } => {
            let bytes = hex::decode(stdin::unwrap_line(value)?).wrap_err("Could not decode hex")?;
            let value = if as_int {
                U256::decode(&mut &bytes[..])?.to_string()
            } else {
                crate::rlp_converter::Item::decode(&mut &bytes[..])
                    .wrap_err("Could not decode rlp")?
                    .to_string()
            };
            print_scalar(value)?;
        }
        CastSubcommand::ToRlp { value } => {
            let value = stdin::unwrap_line(value)?;
            let val =
                serde_json::from_str(&value).unwrap_or_else(|_| serde_json::Value::String(value));
            let item = crate::rlp_converter::Item::value_to_item(&val)?;
            print_scalar(format!("0x{}", hex::encode(alloy_rlp::encode(item))))?;
        }
        CastSubcommand::ToHex(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            print_scalar(to_base(&value, base_in.as_deref(), "hex")?)?;
        }
        CastSubcommand::ToDec(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            print_scalar(to_base(&value, base_in.as_deref(), "dec")?)?;
        }
        CastSubcommand::ToBase { base: ToBaseArgs { value, base_in }, base_out } => {
            let (value, base_out) = stdin::unwrap2(value, base_out)?;
            print_scalar(to_base(&value, base_in.as_deref(), &base_out)?)?;
        }
        CastSubcommand::ToBytes32 { bytes } => {
            let s = stdin::unwrap_line(bytes)?;
            let s = strip_0x(&s);
            if s.len() > 64 {
                eyre::bail!("string >32 bytes");
            }

            let padded = format!("{s:0<64}");
            print_scalar(padded.parse::<B256>()?.to_string())?;
        }
        CastSubcommand::ToBytesMemory { data } => {
            let data = stdin::unwrap_line(data)?;
            const WORD: usize = 32;

            let data = hex::decode(data).wrap_err("Could not decode hex")?;
            let padded_len = data.len().next_multiple_of(WORD);
            let mut out = Vec::with_capacity(WORD + padded_len);
            out.extend_from_slice(&U256::from(data.len()).to_be_bytes::<WORD>());
            out.extend_from_slice(&data);
            out.resize(WORD + padded_len, 0);
            print_scalar(hex::encode_prefixed(out))?;
        }
        CastSubcommand::Pad { data, right, left: _, len } => {
            let s = stdin::unwrap_line(data)?;
            let s = strip_0x(&s);
            let hex_len = len
                .checked_mul(2)
                .filter(|&h| h <= u16::MAX as usize)
                .ok_or_else(|| eyre::eyre!("len out of range: {len}"))?;

            // Validate input
            if s.len() > hex_len {
                eyre::bail!("input length exceeds target length");
            }
            if !s.chars().all(|c| c.is_ascii_hexdigit()) {
                eyre::bail!("input is not a valid hex");
            }

            print_scalar(if right {
                format!("0x{s:0<hex_len$}")
            } else {
                format!("0x{s:0>hex_len$}")
            })?;
        }
        CastSubcommand::FormatBytes32String { string } => {
            let s = stdin::unwrap_line(string)?;
            let str_bytes: &[u8] = s.as_bytes();
            eyre::ensure!(
                str_bytes.len() <= 32,
                "bytes32 strings must not exceed 32 bytes in length"
            );

            let mut bytes32: [u8; 32] = [0u8; 32];
            bytes32[..str_bytes.len()].copy_from_slice(str_bytes);
            print_scalar(hex::encode_prefixed(bytes32))?;
        }
        CastSubcommand::ParseBytes32String { bytes } => {
            let s = stdin::unwrap_line(bytes)?;
            let bytes = hex::decode(s)?;
            eyre::ensure!(bytes.len() == 32, "expected 32 byte hex-string");
            let len = bytes.iter().take_while(|x| **x != 0).count();
            print_scalar(std::str::from_utf8(&bytes[..len])?)?;
        }
        CastSubcommand::ParseBytes32Address { bytes } => {
            let s = stdin::unwrap_line(bytes)?;
            let s = strip_0x(&s);
            if s.len() != 64 {
                eyre::bail!("expected 64 byte hex-string, got {s}");
            }
            let Some(s) = s.strip_prefix("000000000000000000000000") else {
                eyre::bail!("Not convertible to address, there are non-zero bytes");
            };
            print_scalar(Address::from_str(s)?.to_checksum(None))?;
        }

        // ABI encoding & decoding
        CastSubcommand::DecodeAbi { sig, calldata, input } => {
            print_tokens(&abi_decode_calldata(&sig, &calldata, input, false)?)?;
        }
        CastSubcommand::AbiEncode { sig, packed, args } => {
            let out = if packed {
                // If the signature is a tuple, we need to prefix it to make it a function
                let sig = if sig.trim_start().starts_with('(') { format!("foo{sig}") } else { sig };

                let func = get_func(&sig)?;
                let encoded = encode_function_args_packed(&func, &args).map_err(|e| {
                    eyre::eyre!("Could not ABI encode the function and arguments: {e}")
                })?;
                hex::encode_prefixed(encoded)
            } else {
                let func = get_func(&sig)?;
                let encoded = encode_function_args(&func, &args).map_err(|e| {
                    eyre::eyre!("Could not ABI encode the function and arguments: {e}")
                })?;
                hex::encode_prefixed(&encoded[4..])
            };
            print_scalar(out)?;
        }
        // TODO(json): multi-line output (one line per topic + data field), needs structured object
        // envelope
        CastSubcommand::AbiEncodeEvent { sig, args } => {
            let event = get_event(&sig)?;
            if event.inputs.len() != args.len() {
                eyre::bail!(
                    "encode length mismatch: expected {} types, got {}",
                    event.inputs.len(),
                    args.len(),
                );
            }

            let types = event
                .inputs
                .iter()
                .map(Specifier::<DynSolType>::resolve)
                .collect::<Result<Vec<_>, _>>()?;
            let tokens = std::iter::zip(&types, &args)
                .map(|(ty, arg)| Ok(DynSolType::coerce_str(ty, arg.as_ref())?))
                .collect::<Result<Vec<_>>>()?;

            let mut topics = if event.anonymous { vec![] } else { vec![event.selector()] };
            // Non-indexed parameters are encoded together as the event body.
            let mut data_tokens = Vec::new();
            for (input, token) in event.inputs.iter().zip(tokens) {
                if input.indexed {
                    topics.push(encode_event_topic(&token));
                } else {
                    data_tokens.push(token);
                }
            }

            let data = DynSolValue::Tuple(data_tokens).abi_encode_params();
            let log_data = LogData::new_unchecked(topics, data.into());
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
            print_tokens(&abi_decode_calldata(&sig, &raw_hex, true, true)?)?;
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
            print_scalar(hex::encode_prefixed(encode_function_args(&get_func(&sig)?, &args)?))?;
        }
        CastSubcommand::DecodeString { data } => {
            print_tokens(&abi_decode_calldata("Any(string)", &data, true, true)?)?;
        }
        CastSubcommand::DecodeEvent { sig, data } => {
            let decoded_event = if let Some(event_sig) = sig {
                let event = get_event(&event_sig)?;
                event.decode_log_parts(core::iter::once(event.selector()), &hex::decode(data)?)?
            } else {
                let data = strip_0x(&data);
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
                let data = strip_0x(&data);
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
                    let (selector, signature) = get_selector(&sig, opt)?;
                    sh_status!("Successfully generated in {:?}", start_time.elapsed())?;
                    sh_println!("Selector: {selector}")?;
                    sh_println!("Optimized signature: {signature}")?;
                }
                None => print_scalar(get_selector(&sig, 0)?.0)?,
            }
        }

        // Blockchain & RPC queries
        CastSubcommand::AccessList(cmd) => cmd.run().await?,
        CastSubcommand::Age { block, rpc } => {
            let timestamp = rpc_provider(&rpc)?
                .get_block(block.unwrap_or_default())
                .await?
                .ok_or_eyre("block not found")?
                .header
                .timestamp;
            let age = i64::try_from(timestamp)
                .ok()
                .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
                .ok_or_eyre("invalid timestamp")?
                .format("%a %b %e %H:%M:%S %Y");
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
                        ParseUnits::U256(value).format_units(Unit::ETHER)
                    } else {
                        value.to_string()
                    };
                    print_scalar(out)?;
                }
            }
        }
        CastSubcommand::BaseFee { block, rpc } => {
            let fee = rpc_provider(&rpc)?
                .get_block(block.unwrap_or_default())
                .await?
                .ok_or_eyre("block not found")?
                .header
                .base_fee_per_gas
                .ok_or_eyre("base fee not found")?;
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
                    |provider| {
                        let block_id = block;
                        let block = provider
                            .get_block(block_id)
                            .kind(full.into())
                            .await?
                            .ok_or_else(|| eyre::eyre!("block {:?} not found", block_id))?;
                        hex::encode_prefixed(alloy_rlp::encode(block.header().as_ref()))
                    }
                )
            } else {
                let provider = utils::get_provider(&config)?;
                if fields.contains(&"transactions".into()) && !full {
                    eyre::bail!("use --full to view transactions");
                }

                let block = provider
                    .get_block(block)
                    .kind(full.into())
                    .await?
                    .ok_or_else(|| eyre::eyre!("block {:?} not found", block))?;

                if !fields.is_empty() {
                    let mut result = String::new();
                    for field in fields {
                        result.push_str(
                            &get_pretty_block_attr::<alloy_network::AnyNetwork>(&block, &field)
                                .unwrap_or_else(|| format!("{field} is not a valid block field")),
                        );

                        result.push('\n');
                    }
                    result.trim_end().to_string()
                } else if shell::is_json() {
                    serde_json::to_value(&block).unwrap().to_string()
                } else {
                    block.pretty()
                }
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
            let provider = rpc_provider(&rpc)?;
            const GENESIS_CHAINS: &[(&str, &str)] = &[
                ("0xa3c565fc15c7478862d50ccd6561e3c06b24cc509bf388941c25ea985ce32cb9", "kovan"),
                ("0x41941023680923e0fe4d74a34bdac8141f2540e3ae90623718e47d66d1ca4a2d", "ropsten"),
                (
                    "0x7ca38a1916c42007829c55e69d3e9a73265554b586a499015373241b8a3fa48b",
                    "optimism-mainnet",
                ),
                (
                    "0xc1fc15cd51159b1f1e5cbc4b82e85c1447ddfa33c52cf1d98d14fba0d6354be1",
                    "optimism-goerli",
                ),
                (
                    "0x02adc9b449ff5f2467b8c674ece7ff9b21319d76c4ad62a67a70d552655927e5",
                    "optimism-kovan",
                ),
                ("0x521982bd54239dc71269eefb58601762cc15cfb2978e0becb46af7962ed6bfaa", "fraxtal"),
                (
                    "0x910f5c4084b63fd860d0c2f9a04615115a5a991254700b39ba072290dbd77489",
                    "fraxtal-testnet",
                ),
                (
                    "0x7ee576b35482195fc49205cec9af72ce14f003b9ae69f6ba0faef4514be8b442",
                    "arbitrum-mainnet",
                ),
                ("0x0cd786a2425d16f152c658316c423e6ce1181e15c3295826d7c9904cba9ce303", "morden"),
                ("0x6341fd3daf94b748c72ced5a5b26028f2474f5f00d824504e4fa37a75767e177", "rinkeby"),
                ("0xbf7e331f7f7c1dd2e05159666b3bf8bc7a8a3a9eb1d518969eab529dd9b88c1a", "goerli"),
                ("0x14c2283285a88fe5fce9bf5c573ab03d6616695d717b12a127188bcacfc743c4", "kotti"),
                (
                    "0xa9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b",
                    "polygon-pos",
                ),
                (
                    "0x7202b2b53c5a0836e773e319d18922cc756dd67432f9a1f65352b61f4406c697",
                    "polygon-pos-amoy-testnet",
                ),
                (
                    "0x81005434635456a16f74ff7023fbe0bf423abbc8a8deb093ffff455c0ad3b741",
                    "polygon-zkevm",
                ),
                (
                    "0x676c1a76a6c5855a32bdf7c61977a0d1510088a4eeac1330466453b3d08b60b9",
                    "polygon-zkevm-cardona-testnet",
                ),
                ("0x4f1dd23188aab3a76b463e4af801b52b1248ef073c648cbdc4c9333d3da79756", "gnosis"),
                ("0xada44fd8d2ecab8b08f256af07ad3e777f17fb434f8f8e678b312f576212ba9a", "chiado"),
                ("0x6d3c66c5357ec91d5c43af47e234a939b22557cbb552dc45bebbceeed90fbe34", "bsctest"),
                ("0x0d21840abff46b96c84b2ac9e10e4f5cdaeb5693cb665db62a2f3b02d2d57b5b", "bsc"),
                ("0x23a2658170ba70d014ba0d0d2709f8fbfe2fa660cd868c5f282f991eecbe38ee", "ink"),
                (
                    "0xe5fd5cf0be56af58ad5751b401410d6b7a09d830fa459789746a3d0dd1c79834",
                    "ink-sepolia",
                ),
            ];

            let genesis_hash = provider
                .get_block_by_number(0.into())
                .await?
                .ok_or_eyre("block not found")?
                .header
                .hash
                .to_string();
            let chain = match genesis_hash.as_str() {
                // Ethereum and Ethereum Classic share the genesis block and split at the DAO fork.
                "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3" => {
                    match provider
                        .get_block_by_number(1920000.into())
                        .await?
                        .ok_or_eyre("block not found")?
                        .header
                        .hash
                        .to_string()
                        .as_str()
                    {
                        "0x94365e3a8c0b35089c1d1195081fe7489b528a84b22199c916180db8b28ade7f" => {
                            "etclive"
                        }
                        _ => "ethlive",
                    }
                }
                // Avalanche and Fuji share the genesis block.
                "0x31ced5b9beb7f8782b014660da0cb18cc409f121f408186886e1ca3e8eeca96b" => {
                    match provider
                        .get_block_by_number(1.into())
                        .await?
                        .ok_or_eyre("block not found")?
                        .header
                        .hash
                        .to_string()
                        .as_str()
                    {
                        "0x738639479dc82d199365626f90caa82f7eafcfe9ed354b456fb3d294597ceb53" => {
                            "avalanche-fuji"
                        }
                        _ => "avalanche",
                    }
                }
                hash => GENESIS_CHAINS
                    .iter()
                    .find(|(genesis, _)| *genesis == hash)
                    .map_or("unknown", |(_, chain)| chain),
            };
            print_scalar(chain)?;
        }
        CastSubcommand::ChainId { rpc } => {
            print_scalar(rpc_provider(&rpc)?.get_chain_id().await?.to_string())?;
        }
        CastSubcommand::Client { rpc } => {
            print_scalar(rpc_provider(&rpc)?.get_client_version().await?)?;
        }
        CastSubcommand::Code { block, who, disassemble, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            let code = provider.get_code_at(who).block_id(block.unwrap_or_default()).await?;
            print_scalar(if disassemble {
                crate::cmd::disassemble(&code)?
            } else {
                code.to_string()
            })?;
        }
        CastSubcommand::Codesize { block, who, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(
                provider
                    .get_code_at(who)
                    .block_id(block.unwrap_or_default())
                    .await?
                    .len()
                    .to_string(),
            )?;
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
                let nonce = match nonce {
                    Some(nonce) => nonce,
                    None => rpc_provider(&rpc)?.get_transaction_count(address).await?,
                };
                address.create(nonce)
            };
            print_scalar(computed.to_checksum(None))?;
        }
        CastSubcommand::Disassemble { bytecode } => {
            let bytecode = stdin::unwrap_line(bytecode)?;
            print_scalar(crate::cmd::disassemble(&hex::decode(bytecode)?)?)?;
        }
        CastSubcommand::Selectors { bytecode, resolve } => {
            let bytecode = stdin::unwrap_line(bytecode)?;
            let code = hex::decode(&bytecode)?;
            let info = evmole::contract_info(
                evmole::ContractInfoArgs::new(&code)
                    .with_selectors()
                    .with_arguments()
                    .with_state_mutability(),
            );
            let functions = info
                .functions
                .expect("functions extraction was requested")
                .into_iter()
                .filter(|f| f.dispatch == evmole::SelectorDispatch::Abi)
                .map(|f| {
                    let arguments = f
                        .arguments
                        .expect("arguments extraction was requested")
                        .iter()
                        .map(|t| t.sol_type_name())
                        .collect::<Vec<_>>()
                        .join(",");
                    let mutability =
                        f.state_mutability.expect("state_mutability extraction was requested");
                    (
                        alloy_primitives::Selector::from(f.selector),
                        arguments,
                        mutability.as_json_str(),
                    )
                })
                .collect::<Vec<_>>();

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
            print_scalar(rpc_provider(&rpc)?.get_gas_price().await?.to_string())?;
        }
        CastSubcommand::Index { key_type, key, slot_number } => {
            let mut hasher = Keccak256::new();

            let k_ty = DynSolType::parse(&key_type).wrap_err("Could not parse type")?;
            let k = k_ty.coerce_str(&key).wrap_err("Could not parse value")?;
            match k_ty {
                // For value types, `h` pads the value to 32 bytes in the same way as when storing
                // the value in memory.
                DynSolType::Bool
                | DynSolType::Int(_)
                | DynSolType::Uint(_)
                | DynSolType::FixedBytes(_)
                | DynSolType::Address
                | DynSolType::Function => hasher.update(k.as_word().unwrap()),

                // For strings and byte arrays, `h(k)` is just the unpadded data.
                DynSolType::String | DynSolType::Bytes => hasher.update(k.as_packed_seq().unwrap()),

                DynSolType::Array(..)
                | DynSolType::FixedArray(..)
                | DynSolType::Tuple(..)
                | DynSolType::CustomStruct { .. } => {
                    eyre::bail!("Type `{k_ty}` is not supported as a mapping key");
                }
            }

            let p = DynSolType::Uint(256)
                .coerce_str(&slot_number)
                .wrap_err("Could not parse slot number")?;
            let p = p.as_word().unwrap();
            hasher.update(p);

            let location = hasher.finalize();
            print_scalar(location.to_string())?;
        }
        CastSubcommand::IndexErc7201 { id, formula_id } => {
            eyre::ensure!(formula_id == "erc7201", "unsupported formula ID: {formula_id}");
            let id = stdin::unwrap_line(id)?;
            print_scalar(foundry_common::erc7201(&id).to_string())?;
        }
        CastSubcommand::Implementation { block, beacon, who, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            // bytes32(uint256(keccak256('eip1967.proxy.beacon')) - 1)
            const BEACON_SLOT: B256 =
                b256!("0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50");
            // bytes32(uint256(keccak256('eip1967.proxy.implementation')) - 1)
            const IMPLEMENTATION_SLOT: B256 =
                b256!("0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc");

            let slot = if beacon { BEACON_SLOT } else { IMPLEMENTATION_SLOT };
            print_scalar(address_at_slot(&provider, who, slot, block).await?)?;
        }
        CastSubcommand::Admin { block, who, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            // bytes32(uint256(keccak256('eip1967.proxy.admin')) - 1)
            const ADMIN_SLOT: B256 =
                b256!("0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103");
            print_scalar(address_at_slot(&provider, who, ADMIN_SLOT, block).await?)?;
        }
        CastSubcommand::Nonce { block, who, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(
                provider.get_transaction_count(who).block_id(block.unwrap_or_default()).await?,
            )?;
        }
        CastSubcommand::Codehash { block, who, slots, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(
                provider
                    .get_proof(who, slots)
                    .block_id(block.unwrap_or_default())
                    .await?
                    .code_hash
                    .to_string(),
            )?;
        }
        CastSubcommand::StorageRoot { block, who, slots, rpc } => {
            let (provider, who) = rpc_provider_and_address(&rpc, who).await?;
            print_scalar(
                provider
                    .get_proof(who, slots)
                    .block_id(block.unwrap_or_default())
                    .await?
                    .storage_hash
                    .to_string(),
            )?;
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
            let raw_tx = hex::decode(strip_0x(&raw_tx))?;
            let pending_tx = provider.send_raw_transaction(&raw_tx).await?;
            if cast_async {
                print_scalar(format!("{:#x}", pending_tx.inner().tx_hash()))?;
            } else {
                print_json_object(pending_tx.get_receipt().await?)?;
            }
        }
        CastSubcommand::Receipt { tx_hash, field, cast_async, confirmations, rpc } => {
            // JSON: The receipt helper already formats the output.
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
                            transaction_response(&provider, tx_hash, from, nonce).await?;
                        tx.as_ref().encoded_2718().into()
                    },
                    _ => {
                        let provider = utils::get_provider(&config)?;
                        let tx =
                            transaction_response(&provider, tx_hash, from, nonce).await?;
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
                        let tx = transaction_response(&provider, tx_hash, from, nonce).await?;
                        format_transaction(&provider, tx, field, to_request)?
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

            print_tokens(&abi_decode_calldata(sig, &calldata, true, true)?)?;
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
                Ok(s) => {
                    // Hex-decode if data starts with 0x.
                    if s.starts_with("0x") {
                        keccak256(hex::decode(s.trim_end())?)
                    } else {
                        keccak256(s)
                    }
                    .to_string()
                }
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
            print_scalar(shift(&value, &bits, base_in.as_deref(), &base_out, |value, bits| {
                value << bits
            })?)?;
        }
        CastSubcommand::RightShift { value, bits, base_in, base_out } => {
            print_scalar(shift(&value, &bits, base_in.as_deref(), &base_out, |value, bits| {
                value.wrapping_shr(bits.saturating_to())
            })?)?;
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
            let client = explorer_client(chain, api_key, explorer_api_url, explorer_url)?;
            let metadata = client.contract_source_code(address.parse()?).await?;
            match (directory, flatten) {
                (Some(dir), false) => {
                    metadata.source_tree().write_to(&dir)?;
                }
                (None, false) => sh_println!("{}", metadata.source_code())?,
                (dir, true) => {
                    let Some(metadata) = metadata.items.first() else {
                        eyre::bail!("Empty contract source code");
                    };

                    let tmp = tempfile::tempdir()?;
                    let project = foundry_common::compile::etherscan_project(metadata, tmp.path())?;
                    let target_path = project.find_contract_path(&metadata.contract_name)?;

                    let flattened = foundry_common::flatten(project, &target_path)?;

                    if let Some(path) = dir {
                        fs::create_dir_all(path.parent().unwrap())?;
                        fs::write(&path, flattened)?;
                        sh_status!("Flattened file written at {}", path.display())?
                    } else {
                        sh_println!("{flattened}")?
                    }
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
                Some(NetworkVariant::Optimism) => decode_raw_transaction::<Optimism>(&tx)?,
                Some(NetworkVariant::Tempo) => decode_raw_transaction::<TempoNetwork>(&tx)?,
                Some(NetworkVariant::Ethereum) => decode_raw_transaction::<Ethereum>(&tx)?,
                #[cfg(feature = "monad")]
                Some(NetworkVariant::Monad) => decode_raw_transaction::<Ethereum>(&tx)?,
                // Without an explicit `--network` override, decode with the Foundry envelope,
                // which dispatches on the EIP-2718 type byte for the transaction types compiled
                // into `FoundryNetwork`, including Tempo txs (`0x76`).
                None => decode_raw_transaction::<FoundryNetwork>(&tx)?,
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

async fn address_at_slot<N: alloy_network::Network>(
    provider: &impl Provider<N>,
    who: Address,
    slot: B256,
    block: Option<BlockId>,
) -> Result<String> {
    let value =
        provider.get_storage_at(who, slot.into()).block_id(block.unwrap_or_default()).await?;
    Ok(format!("{:?}", Address::from_word(value.into())))
}

async fn transaction_response<N: Network>(
    provider: &impl Provider<N>,
    tx_hash: Option<String>,
    from: Option<NameOrAddress>,
    nonce: Option<u64>,
) -> Result<N::TransactionResponse> {
    if let Some(tx_hash) = tx_hash {
        let tx_hash = TxHash::from_str(&tx_hash).wrap_err("invalid tx hash")?;
        provider
            .get_transaction_by_hash(tx_hash)
            .await?
            .ok_or_else(|| eyre::eyre!("tx not found: {:?}", tx_hash))
    } else if let Some(from) = from {
        let nonce = U64::from(nonce.unwrap_or_default());
        let from = from.resolve(provider.root()).await?;
        provider
            .raw_request::<_, Option<N::TransactionResponse>>(
                "eth_getTransactionBySenderAndNonce".into(),
                (from, nonce),
            )
            .await?
            .ok_or_else(|| {
                eyre::eyre!("tx not found for sender {from} and nonce {:?}", nonce.to::<u64>())
            })
    } else {
        eyre::bail!("tx hash or from address is required")
    }
}

fn format_transaction<N: Network>(
    _provider: &impl Provider<N>,
    tx: N::TransactionResponse,
    field: Option<String>,
    to_request: bool,
) -> Result<String>
where
    N::TransactionResponse: UIfmt,
    N::TxEnvelope: UIfmtSignatureExt,
{
    Ok(if let Some(field) = field {
        if let Some(value) = get_pretty_tx_attr::<N>(&tx, &field) {
            value
        } else {
            let tx_json = serde_json::to_value(&tx)?;
            let value =
                tx_json.get(&field).ok_or_else(|| eyre::eyre!("invalid tx field: {field}"))?;
            match value {
                serde_json::Value::String(value) => value.clone(),
                value => value.to_string(),
            }
        }
    } else if shell::is_json() {
        // to_value first to sort json object keys
        serde_json::to_value(&tx)?.to_string()
    } else if to_request {
        serde_json::to_string_pretty(&Into::<N::TransactionRequest>::into(tx))?
    } else {
        tx.pretty()
    })
}

fn int_bound(s: &str, max: bool) -> Result<String> {
    let ty = DynSolType::parse(s).wrap_err("Invalid type, expected `(u)int<bit size>`")?;
    match ty {
        DynSolType::Int(n) => {
            let max_value = (U256::MAX & U256::from(1).wrapping_shl(n - 1)) - U256::from(1);
            if max {
                Ok(max_value.to_string())
            } else {
                Ok((I256::from_raw(max_value).wrapping_neg() + I256::MINUS_ONE).to_string())
            }
        }
        DynSolType::Uint(n) if max => {
            let mut max_value = U256::MAX;
            if n < 256 {
                max_value &= U256::from(1).wrapping_shl(n).wrapping_sub(U256::from(1));
            }
            Ok(max_value.to_string())
        }
        DynSolType::Uint(_) => Ok("0".to_string()),
        _ => Err(eyre::eyre!("Type is not int/uint: {s}")),
    }
}

/// Converts a parsed, possibly-negative [`NumberWithBase`] into a [`ParseUnits`], preserving
/// its sign.
///
/// `NumberWithBase::number()` returns the two's-complement bits of a negative value modulo
/// 2^256, which is a wider range than [`I256`] can represent (magnitudes up to 2^255 only).
/// A magnitude beyond that range would silently reinterpret as a small *positive* [`I256`]
/// if constructed unconditionally via [`I256::from_raw`] -- reject it instead.
fn signed_parse_units(value: &NumberWithBase) -> Result<ParseUnits> {
    if value.is_nonnegative() {
        return Ok(ParseUnits::U256(value.number()));
    }
    let signed = I256::from_raw(value.number());
    if !signed.is_negative() {
        eyre::bail!("value out of range for a signed 256-bit integer");
    }
    Ok(ParseUnits::I256(signed))
}

fn format_unit_as_string(value: ParseUnits, unit: Unit) -> String {
    let mut formatted = value.format_units(unit);
    // Trim empty fractional part.
    if let Some(dot) = formatted.find('.') {
        let fractional = &formatted[dot + 1..];
        if fractional.chars().all(|c: char| c == '0') {
            formatted = formatted[..dot].to_string();
        }
    }
    formatted
}

pub(super) fn format_units(value: &str, unit: u8) -> Result<String> {
    let value = NumberWithBase::parse_int(value, None)?;
    let unit = Unit::new(unit).ok_or_else(|| eyre::eyre!("invalid unit"))?;
    let parsed = signed_parse_units(&value)?;
    Ok(format_unit_as_string(parsed, unit))
}

fn to_base(value: &str, base_in: Option<&str>, base_out: &str) -> Result<String> {
    let base_in = Base::unwrap_or_detect(base_in, value)?;
    let base_out = base_out.parse()?;
    if base_in == base_out {
        return Ok(value.to_string());
    }
    let n = NumberWithBase::parse_int_in(value, base_in)?.with_base(base_out);
    Ok(format!("{n:#?}"))
}

/// Parses `value` and `bits`, applies `shift` and formats the result with the `base_out`
/// prefix.
fn shift(
    value: &str,
    bits: &str,
    base_in: Option<&str>,
    base_out: &str,
    shift: impl FnOnce(U256, U256) -> U256,
) -> Result<String> {
    let base_out = base_out.parse()?;
    let value = NumberWithBase::parse_uint(value, base_in)?.number();
    let bits = NumberWithBase::parse_uint(bits, None)?.number();
    Ok(format!("{:#?}", NumberWithBase::from(shift(value, bits)).with_base(base_out)))
}

fn explorer_client(
    chain: Chain,
    api_key: Option<String>,
    api_url: Option<String>,
    explorer_url: Option<String>,
) -> Result<Client> {
    let mut builder = Client::builder();

    let deduced = chain.etherscan_urls();

    let explorer_url = explorer_url
        .or(deduced.map(|d| d.1.to_string()))
        .ok_or_eyre("Please provide the explorer browser URL using `--explorer-url`")?;
    builder = builder.with_url(explorer_url)?;

    let api_url = api_url
        .or(deduced.map(|d| d.0.to_string()))
        .ok_or_eyre("Please provide the explorer API URL using `--explorer-api-url`")?;
    builder = builder.with_api_url(api_url)?;

    if let Some(api_key) = api_key {
        builder = builder.with_api_key(api_key);
    }

    builder.build().map_err(Into::into)
}

fn decode_raw_transaction<N: Network<TxEnvelope: SignerRecoverable + Serialize>>(
    tx: &str,
) -> Result<String> {
    let tx_hex = hex::decode(tx)?;
    let tx: N::TxEnvelope = Decodable2718::decode_2718(&mut tx_hex.as_slice())?;
    if let Ok(signer) = tx.recover_signer() {
        Ok(serde_json::to_string_pretty(&Recovered::new_unchecked(tx, signer))?)
    } else {
        Ok(serde_json::to_string_pretty(&tx)?)
    }
}

fn get_selector(signature: &str, optimize: usize) -> Result<(String, String)> {
    if optimize > 4 {
        eyre::bail!("number of leading zeroes must not be greater than 4");
    }
    if optimize == 0 {
        let selector = get_func(signature)?.selector();
        return Ok((selector.to_string(), String::from(signature)));
    }
    let Some((name, params)) = signature.split_once('(') else {
        eyre::bail!("invalid function signature");
    };

    let num_threads = rayon::current_num_threads();
    let found = AtomicBool::new(false);

    // Each thread walks its own residue class of nonces until one of them finds a match.
    (0..num_threads as u32)
        .into_par_iter()
        .find_map_any(|mut nonce| {
            while nonce < u32::MAX && !found.load(Ordering::Relaxed) {
                let input = format!("{name}{nonce}({params}");
                let selector = &keccak256(input.as_bytes())[..4];
                if selector.iter().take_while(|&&byte| byte == 0).count() == optimize {
                    found.store(true, Ordering::Relaxed);
                    return Some((hex::encode_prefixed(selector), input));
                }
                nonce += num_threads as u32;
            }
            None
        })
        .ok_or_eyre("No selector found")
}

fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x").unwrap_or(s)
}

/// Encodes the topic of an indexed event parameter.
///
/// Value types are encoded as their 32-byte word. Reference types are hashed over the special
/// in-place encoding defined for indexed event parameters, which differs from regular ABI
/// encoding: `string` and `bytes` contribute their raw contents, and array or struct members are
/// concatenated recursively without any offsets or length prefixes.
///
/// See <https://docs.soliditylang.org/en/latest/abi-spec.html#encoding-of-indexed-event-parameters>
pub(super) fn encode_event_topic(value: &DynSolValue) -> B256 {
    if let Some(word) = value.as_word() {
        return word;
    }
    // Top-level `string` and `bytes` hash their raw contents without padding.
    if let Some(bytes) = value.as_packed_seq() {
        return keccak256(bytes);
    }
    let mut preimage = Vec::new();
    encode_event_topic_preimage(value, &mut preimage);
    keccak256(preimage)
}

/// Encodes a value into the in-place preimage of an indexed event parameter: words as-is,
/// `string`/`bytes` right-padded to a multiple of 32 bytes, and sequences as the concatenation of
/// their encoded members.
fn encode_event_topic_preimage(value: &DynSolValue, out: &mut Vec<u8>) {
    if let Some(word) = value.as_word() {
        out.extend_from_slice(word.as_slice());
    } else if let Some(bytes) = value.as_packed_seq() {
        let pad = bytes.len().next_multiple_of(32) - bytes.len();
        out.extend_from_slice(bytes);
        out.resize(out.len() + pad, 0);
    } else if let Some(values) = value.as_fixed_seq().or_else(|| value.as_array()) {
        for value in values {
            encode_event_topic_preimage(value, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::{EventTopic, sol_data};

    /// Compares [`super::encode_event_topic`] against alloy's static [`EventTopic`]
    /// implementation, which `sol!`-generated events use to compute indexed topics.
    #[test]
    fn encode_event_topic_matches_static_encoding() {
        let uint = |n: u64| DynSolValue::Uint(U256::from(n), 256);
        let string = |s: &str| DynSolValue::String(s.into());
        let topic = |v: &DynSolValue| super::encode_event_topic(v);

        let long = "abcdefghijklmnopqrstuvwxyz0123456789abcd";
        for s in ["", "hello", long] {
            assert_eq!(
                topic(&string(s)),
                <sol_data::String as EventTopic>::encode_topic(&s.to_string()).0,
                "string {s:?}"
            );
        }

        let bytes = hex::decode("deadbeef").unwrap();
        assert_eq!(
            topic(&DynSolValue::Bytes(bytes.clone())),
            <sol_data::Bytes as EventTopic>::encode_topic(&Bytes::from(bytes)).0,
        );

        let addr = Address::repeat_byte(0x42);
        assert_eq!(
            topic(&DynSolValue::Address(addr)),
            <sol_data::Address as EventTopic>::encode_topic(&addr).0,
        );

        assert_eq!(
            topic(&DynSolValue::Array(vec![uint(1), uint(2)])),
            <sol_data::Array<sol_data::Uint<256>> as EventTopic>::encode_topic(&vec![
                U256::from(1),
                U256::from(2)
            ])
            .0,
        );

        assert_eq!(
            topic(&DynSolValue::FixedArray(vec![uint(7), uint(9)])),
            <sol_data::FixedArray<sol_data::Uint<256>, 2> as EventTopic>::encode_topic(&[
                U256::from(7),
                U256::from(9)
            ])
            .0,
        );

        assert_eq!(
            topic(&DynSolValue::Array(vec![string("alpha"), string(long)])),
            <sol_data::Array<sol_data::String> as EventTopic>::encode_topic(&vec![
                "alpha".to_string(),
                long.to_string()
            ])
            .0,
        );

        assert_eq!(
            topic(&DynSolValue::Tuple(vec![uint(7), string("hello")])),
            <(sol_data::Uint<256>, sol_data::String) as EventTopic>::encode_topic(&(
                U256::from(7),
                "hello".to_string()
            ))
            .0,
        );

        assert_eq!(
            topic(&DynSolValue::Array(vec![
                DynSolValue::Array(vec![uint(1)]),
                DynSolValue::Array(vec![uint(2), uint(3)]),
            ])),
            <sol_data::Array<sol_data::Array<sol_data::Uint<256>>> as EventTopic>::encode_topic(
                &vec![vec![U256::from(1)], vec![U256::from(2), U256::from(3)]]
            )
            .0,
        );
    }
}
