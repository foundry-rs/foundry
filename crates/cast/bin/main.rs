#[macro_use]
extern crate tracing;

use alloy_primitives::{eip191_hash_message, hex, keccak256, Address, B256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag::Latest};
use cast::{Cast, SimpleCast};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use eyre::Result;
use foundry_cli::{handler, prompt, stdin, utils};
use foundry_common::{
    abi::get_event,
    ens::{namehash, ProviderEnsExt},
    fmt::{format_uint_exp, print_tokens},
    fs,
    selectors::{
        decode_calldata, decode_event_topic, decode_function_selector, decode_selectors,
        import_selectors, parse_signatures, pretty_calldata, ParsedSignatures, SelectorImportData,
        SelectorType,
    },
};
use foundry_config::Config;
use std::time::Instant;

pub mod args;
pub mod cmd;
pub mod tx;

use args::{Cast as CastArgs, CastSubcommand, ToBaseArgs};

#[cfg(all(feature = "jemalloc", unix))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() -> Result<()> {
    handler::install();
    utils::load_dotenv();
    utils::subscriber();
    utils::enable_paint();
    let args = CastArgs::parse();
    main_args(args)
}

#[allow(clippy::needless_return)]
#[tokio::main]
async fn main_args(args: CastArgs) -> Result<()> {
    match args.cmd {
        // Constants
        CastSubcommand::MaxInt { r#type } => {
            println!("{}", SimpleCast::max_int(&r#type)?);
        }
        CastSubcommand::MinInt { r#type } => {
            println!("{}", SimpleCast::min_int(&r#type)?);
        }
        CastSubcommand::MaxUint { r#type } => {
            println!("{}", SimpleCast::max_int(&r#type)?);
        }
        CastSubcommand::AddressZero => {
            println!("{:?}", Address::ZERO);
        }
        CastSubcommand::HashZero => {
            println!("{:?}", B256::ZERO);
        }

        // Conversions & transformations
        CastSubcommand::FromUtf8 { text } => {
            let value = stdin::unwrap(text, false)?;
            println!("{}", SimpleCast::from_utf8(&value));
        }
        CastSubcommand::ToAscii { hexdata } => {
            let value = stdin::unwrap(hexdata, false)?;
            println!("{}", SimpleCast::to_ascii(value.trim())?);
        }
        CastSubcommand::ToUtf8 { hexdata } => {
            let value = stdin::unwrap(hexdata, false)?;
            println!("{}", SimpleCast::to_utf8(&value)?);
        }
        CastSubcommand::FromFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;
            println!("{}", SimpleCast::from_fixed_point(&value, &decimals)?);
        }
        CastSubcommand::ToFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;
            println!("{}", SimpleCast::to_fixed_point(&value, &decimals)?);
        }
        CastSubcommand::ConcatHex { data } => {
            if data.is_empty() {
                let s = stdin::read(true)?;
                println!("{}", SimpleCast::concat_hex(s.split_whitespace()))
            } else {
                println!("{}", SimpleCast::concat_hex(data))
            }
        }
        CastSubcommand::FromBin => {
            let hex = stdin::read_bytes(false)?;
            println!("{}", hex::encode_prefixed(hex));
        }
        CastSubcommand::ToHexdata { input } => {
            let value = stdin::unwrap_line(input)?;
            let output = match value {
                s if s.starts_with('@') => hex::encode(std::env::var(&s[1..])?),
                s if s.starts_with('/') => hex::encode(fs::read(s)?),
                s => s.split(':').map(|s| s.trim_start_matches("0x").to_lowercase()).collect(),
            };
            println!("0x{output}");
        }
        CastSubcommand::ToCheckSumAddress { address } => {
            let value = stdin::unwrap_line(address)?;
            println!("{}", value.to_checksum(None));
        }
        CastSubcommand::ToUint256 { value } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_uint256(&value)?);
        }
        CastSubcommand::ToInt256 { value } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_int256(&value)?);
        }
        CastSubcommand::ToUnit { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_unit(&value, &unit)?);
        }
        CastSubcommand::FromWei { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::from_wei(&value, &unit)?);
        }
        CastSubcommand::ToWei { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_wei(&value, &unit)?);
        }
        CastSubcommand::FromRlp { value } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::from_rlp(value)?);
        }
        CastSubcommand::ToRlp { value } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_rlp(&value)?);
        }
        CastSubcommand::ToHex(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_base(&value, base_in.as_deref(), "hex")?);
        }
        CastSubcommand::ToDec(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_base(&value, base_in.as_deref(), "dec")?);
        }
        CastSubcommand::ToBase { base: ToBaseArgs { value, base_in }, base_out } => {
            let (value, base_out) = stdin::unwrap2(value, base_out)?;
            println!("{}", SimpleCast::to_base(&value, base_in.as_deref(), &base_out)?);
        }
        CastSubcommand::ToBytes32 { bytes } => {
            let value = stdin::unwrap_line(bytes)?;
            println!("{}", SimpleCast::to_bytes32(&value)?);
        }
        CastSubcommand::FormatBytes32String { string } => {
            let value = stdin::unwrap_line(string)?;
            println!("{}", SimpleCast::format_bytes32_string(&value)?);
        }
        CastSubcommand::ParseBytes32String { bytes } => {
            let value = stdin::unwrap_line(bytes)?;
            println!("{}", SimpleCast::parse_bytes32_string(&value)?);
        }
        CastSubcommand::ParseBytes32Address { bytes } => {
            let value = stdin::unwrap_line(bytes)?;
            println!("{}", SimpleCast::parse_bytes32_address(&value)?);
        }

        // ABI encoding & decoding
        CastSubcommand::AbiDecode { sig, calldata, input, json } => {
            let tokens = SimpleCast::abi_decode(&sig, &calldata, input)?;
            print_tokens(&tokens, json)
        }
        CastSubcommand::AbiEncode { sig, packed, args } => {
            if !packed {
                println!("{}", SimpleCast::abi_encode(&sig, &args)?);
            } else {
                println!("{}", SimpleCast::abi_encode_packed(&sig, &args)?);
            }
        }
        CastSubcommand::CalldataDecode { sig, calldata, json } => {
            let tokens = SimpleCast::calldata_decode(&sig, &calldata, true)?;
            print_tokens(&tokens, json)
        }
        CastSubcommand::CalldataEncode { sig, args } => {
            println!("{}", SimpleCast::calldata_encode(sig, &args)?);
        }
        CastSubcommand::Interface(cmd) => cmd.run().await?,
        CastSubcommand::Bind(cmd) => cmd.run().await?,
        CastSubcommand::PrettyCalldata { calldata, offline } => {
            let calldata = stdin::unwrap_line(calldata)?;
            println!("{}", pretty_calldata(&calldata, offline).await?);
        }
        CastSubcommand::Sig { sig, optimize } => {
            let sig = stdin::unwrap_line(sig)?;
            match optimize {
                Some(opt) => {
                    println!("Starting to optimize signature...");
                    let start_time = Instant::now();
                    let (selector, signature) = SimpleCast::get_selector(&sig, opt)?;
                    println!("Successfully generated in {:?}", start_time.elapsed());
                    println!("Selector: {selector}");
                    println!("Optimized signature: {signature}");
                }
                None => println!("{}", SimpleCast::get_selector(&sig, 0)?.0),
            }
        }

        // Blockchain & RPC queries
        CastSubcommand::AccessList(cmd) => cmd.run().await?,
        CastSubcommand::Age { block, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!(
                "{}",
                Cast::new(provider).age(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        CastSubcommand::Balance { block, who, ether, rpc, erc20 } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let account_addr = who.resolve(&provider).await?;

            match erc20 {
                Some(token) => {
                    let balance =
                        Cast::new(&provider).erc20_balance(token, account_addr, block).await?;
                    println!("{}", format_uint_exp(balance));
                }
                None => {
                    let value = Cast::new(&provider).balance(account_addr, block).await?;
                    if ether {
                        println!("{}", SimpleCast::from_wei(&value.to_string(), "eth")?);
                    } else {
                        println!("{value}");
                    }
                }
            }
        }
        CastSubcommand::BaseFee { block, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!(
                "{}",
                Cast::new(provider).base_fee(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        CastSubcommand::Block { block, full, field, json, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!(
                "{}",
                Cast::new(provider)
                    .block(block.unwrap_or(BlockId::Number(Latest)), full, field, json)
                    .await?
            );
        }
        CastSubcommand::BlockNumber { rpc, block } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let number = match block {
                Some(id) => {
                    provider
                        .get_block(id, false.into())
                        .await?
                        .ok_or_else(|| eyre::eyre!("block {id:?} not found"))?
                        .header
                        .number
                }
                None => Cast::new(provider).block_number().await?,
            };
            println!("{number}");
        }
        CastSubcommand::Chain { rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).chain().await?);
        }
        CastSubcommand::ChainId { rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).chain_id().await?);
        }
        CastSubcommand::Client { rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", provider.get_client_version().await?);
        }
        CastSubcommand::Code { block, who, disassemble, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            println!("{}", Cast::new(provider).code(who, block, disassemble).await?);
        }
        CastSubcommand::Codesize { block, who, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            println!("{}", Cast::new(provider).codesize(who, block).await?);
        }
        CastSubcommand::ComputeAddress { address, nonce, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;

            let address: Address = stdin::unwrap_line(address)?.parse()?;
            let computed = Cast::new(provider).compute_address(address, nonce).await?;
            println!("Computed Address: {}", computed.to_checksum(None));
        }
        CastSubcommand::Disassemble { bytecode } => {
            println!("{}", SimpleCast::disassemble(&bytecode)?);
        }
        CastSubcommand::Selectors { bytecode, resolve } => {
            let functions = SimpleCast::extract_functions(&bytecode)?;
            let max_args_len = functions.iter().map(|r| r.1.len()).max().unwrap_or(0);
            let max_mutability_len = functions.iter().map(|r| r.2.len()).max().unwrap_or(0);

            let resolve_results = if resolve {
                let selectors_it = functions.iter().map(|r| &r.0);
                let ds = decode_selectors(SelectorType::Function, selectors_it).await?;
                ds.into_iter().map(|v| v.unwrap_or_default().join("|")).collect()
            } else {
                vec![]
            };
            for (pos, (selector, arguments, state_mutability)) in functions.into_iter().enumerate()
            {
                if resolve {
                    let resolved = &resolve_results[pos];
                    println!("{selector}\t{arguments:max_args_len$}\t{state_mutability:max_mutability_len$}\t{resolved}");
                } else {
                    println!("{selector}\t{arguments:max_args_len$}\t{state_mutability}");
                }
            }
        }
        CastSubcommand::FindBlock(cmd) => cmd.run().await?,
        CastSubcommand::GasPrice { rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).gas_price().await?);
        }
        CastSubcommand::Index { key_type, key, slot_number } => {
            println!("{}", SimpleCast::index(&key_type, &key, &slot_number)?);
        }
        CastSubcommand::IndexErc7201 { id, formula_id } => {
            eyre::ensure!(formula_id == "erc7201", "unsupported formula ID: {formula_id}");
            let id = stdin::unwrap_line(id)?;
            println!("{}", foundry_common::erc7201(&id));
        }
        CastSubcommand::Implementation { block, who, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            println!("{}", Cast::new(provider).implementation(who, block).await?);
        }
        CastSubcommand::Admin { block, who, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            println!("{}", Cast::new(provider).admin(who, block).await?);
        }
        CastSubcommand::Nonce { block, who, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            println!("{}", Cast::new(provider).nonce(who, block).await?);
        }
        CastSubcommand::Codehash { block, who, slots, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            println!("{}", Cast::new(provider).codehash(who, slots, block).await?);
        }
        CastSubcommand::StorageRoot { block, who, slots, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let who = who.resolve(&provider).await?;
            println!("{}", Cast::new(provider).storage_root(who, slots, block).await?);
        }
        CastSubcommand::Proof { address, slots, rpc, block } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let address = address.resolve(&provider).await?;
            let value = provider
                .get_proof(address, slots.into_iter().collect())
                .block_id(block.unwrap_or_default())
                .await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        CastSubcommand::Rpc(cmd) => cmd.run().await?,
        CastSubcommand::Storage(cmd) => cmd.run().await?,

        // Calls & transactions
        CastSubcommand::Call(cmd) => cmd.run().await?,
        CastSubcommand::Estimate(cmd) => cmd.run().await?,
        CastSubcommand::MakeTx(cmd) => cmd.run().await?,
        CastSubcommand::PublishTx { raw_tx, cast_async, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let cast = Cast::new(&provider);
            let pending_tx = cast.publish(raw_tx).await?;
            let tx_hash = pending_tx.inner().tx_hash();

            if cast_async {
                println!("{tx_hash:#x}");
            } else {
                let receipt = pending_tx.get_receipt().await?;
                println!("{}", serde_json::json!(receipt));
            }
        }
        CastSubcommand::Receipt { tx_hash, field, json, cast_async, confirmations, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!(
                "{}",
                Cast::new(provider)
                    .receipt(tx_hash, field, confirmations, None, cast_async, json)
                    .await?
            );
        }
        CastSubcommand::Run(cmd) => cmd.run().await?,
        CastSubcommand::SendTx(cmd) => cmd.run().await?,
        CastSubcommand::Tx { tx_hash, field, raw, json, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;

            // Can use either --raw or specify raw as a field
            let raw = raw || field.as_ref().is_some_and(|f| f == "raw");

            println!("{}", Cast::new(&provider).transaction(tx_hash, field, raw, json).await?)
        }

        // 4Byte
        CastSubcommand::FourByte { selector } => {
            let selector = stdin::unwrap_line(selector)?;
            let sigs = decode_function_selector(&selector).await?;
            if sigs.is_empty() {
                eyre::bail!("No matching function signatures found for selector `{selector}`");
            }
            for sig in sigs {
                println!("{sig}");
            }
        }
        CastSubcommand::FourByteDecode { calldata, json } => {
            let calldata = stdin::unwrap_line(calldata)?;
            let sigs = decode_calldata(&calldata).await?;
            sigs.iter().enumerate().for_each(|(i, sig)| println!("{}) \"{sig}\"", i + 1));

            let sig = match sigs.len() {
                0 => eyre::bail!("No signatures found"),
                1 => sigs.first().unwrap(),
                _ => {
                    let i: usize = prompt!("Select a function signature by number: ")?;
                    sigs.get(i - 1).ok_or_else(|| eyre::eyre!("Invalid signature index"))?
                }
            };

            let tokens = SimpleCast::calldata_decode(sig, &calldata, true)?;
            print_tokens(&tokens, json)
        }
        CastSubcommand::FourByteEvent { topic } => {
            let topic = stdin::unwrap_line(topic)?;
            let sigs = decode_event_topic(&topic).await?;
            if sigs.is_empty() {
                eyre::bail!("No matching event signatures found for topic `{topic}`");
            }
            for sig in sigs {
                println!("{sig}");
            }
        }
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
            println!("{}", namehash(&name));
        }
        CastSubcommand::LookupAddress { who, rpc, verify } => {
            let config = Config::from(&rpc);
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
            println!("{name}");
        }
        CastSubcommand::ResolveName { who, rpc, verify } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;

            let who = stdin::unwrap_line(who)?;
            let address = provider.resolve_name(&who).await?;
            if verify {
                let name = provider.lookup_address(&address).await?;
                eyre::ensure!(
                    name == who,
                    "Forward lookup verification failed: got `{name}`, expected `{who}`"
                );
            }
            println!("{address}");
        }

        // Misc
        CastSubcommand::Keccak { data } => {
            let bytes = match data {
                Some(data) => data.into_bytes(),
                None => stdin::read_bytes(false)?,
            };
            match String::from_utf8(bytes) {
                Ok(s) => {
                    let s = SimpleCast::keccak(&s)?;
                    println!("{s}");
                }
                Err(e) => {
                    let hash = keccak256(e.as_bytes());
                    let s = hex::encode(hash);
                    println!("0x{s}");
                }
            };
        }
        CastSubcommand::HashMessage { message } => {
            let message = stdin::unwrap_line(message)?;
            let input = match message.strip_prefix("0x") {
                Some(hex_str) => hex::decode(hex_str)?,
                None => message.as_bytes().to_vec(),
            };
            println!("{}", eip191_hash_message(input));
        }
        CastSubcommand::SigEvent { event_string } => {
            let event_string = stdin::unwrap_line(event_string)?;
            let parsed_event = get_event(&event_string)?;
            println!("{:?}", parsed_event.selector());
        }
        CastSubcommand::LeftShift { value, bits, base_in, base_out } => {
            println!("{}", SimpleCast::left_shift(&value, &bits, base_in.as_deref(), &base_out)?);
        }
        CastSubcommand::RightShift { value, bits, base_in, base_out } => {
            println!("{}", SimpleCast::right_shift(&value, &bits, base_in.as_deref(), &base_out)?);
        }
        CastSubcommand::EtherscanSource { address, directory, etherscan, flatten } => {
            let config = Config::from(&etherscan);
            let chain = config.chain.unwrap_or_default();
            let api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();
            match (directory, flatten) {
                (Some(dir), false) => {
                    SimpleCast::expand_etherscan_source_to_directory(chain, address, api_key, dir)
                        .await?
                }
                (None, false) => {
                    println!("{}", SimpleCast::etherscan_source(chain, address, api_key).await?);
                }
                (dir, true) => {
                    SimpleCast::etherscan_source_flatten(chain, address, api_key, dir).await?;
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
        CastSubcommand::GenerateFigSpec => clap_complete::generate(
            clap_complete_fig::Fig,
            &mut CastArgs::command(),
            "cast",
            &mut std::io::stdout(),
        ),
        CastSubcommand::Logs(cmd) => cmd.run().await?,
        CastSubcommand::DecodeTransaction { tx } => {
            let tx = stdin::unwrap_line(tx)?;
            let tx = SimpleCast::decode_raw_transaction(&tx)?;

            println!("{}", serde_json::to_string_pretty(&tx)?);
        }
        CastSubcommand::DecodeEof { eof } => {
            let eof = stdin::unwrap_line(eof)?;
            println!("{}", SimpleCast::decode_eof(&eof)?);
        }
    };
    Ok(())
}
