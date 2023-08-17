#![warn(unused_crate_dependencies)]

use cast::{Cast, SimpleCast};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use ethers::{
    core::types::{BlockId, BlockNumber::Latest, H256},
    providers::Middleware,
    types::Address,
    utils::keccak256,
};
use eyre::Result;
use foundry_cli::{handler, prompt, stdin, utils};
use foundry_common::{
    abi::{format_tokens, get_event},
    fs,
    selectors::{
        decode_calldata, decode_event_topic, decode_function_selector, import_selectors,
        parse_signatures, pretty_calldata, ParsedSignatures, SelectorImportData,
    },
};
use foundry_config::Config;
use std::time::Instant;

pub mod cmd;
pub mod opts;

use opts::{Opts, Subcommands, ToBaseArgs};

#[tokio::main]
async fn main() -> Result<()> {
    utils::load_dotenv();
    handler::install()?;
    utils::subscriber();
    utils::enable_paint();

    let opts = Opts::parse();
    match opts.sub {
        // Constants
        Subcommands::MaxInt { r#type } => {
            println!("{}", SimpleCast::max_int(&r#type)?);
        }
        Subcommands::MinInt { r#type } => {
            println!("{}", SimpleCast::min_int(&r#type)?);
        }
        Subcommands::MaxUint { r#type } => {
            println!("{}", SimpleCast::max_int(&r#type)?);
        }
        Subcommands::AddressZero => {
            println!("{:?}", Address::zero());
        }
        Subcommands::HashZero => {
            println!("{:?}", H256::zero());
        }

        // Conversions & transformations
        Subcommands::FromUtf8 { text } => {
            let value = stdin::unwrap(text, false)?;
            println!("{}", SimpleCast::from_utf8(&value));
        }
        Subcommands::ToAscii { hexdata } => {
            let value = stdin::unwrap(hexdata, false)?;
            println!("{}", SimpleCast::to_ascii(&value)?);
        }
        Subcommands::FromFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;
            println!("{}", SimpleCast::from_fixed_point(&value, &decimals)?);
        }
        Subcommands::ToFixedPoint { value, decimals } => {
            let (value, decimals) = stdin::unwrap2(value, decimals)?;
            println!("{}", SimpleCast::to_fixed_point(&value, &decimals)?);
        }
        Subcommands::ConcatHex { data } => {
            if data.is_empty() {
                let s = stdin::read(true)?;
                println!("{}", SimpleCast::concat_hex(s.split_whitespace()))
            } else {
                println!("{}", SimpleCast::concat_hex(data))
            }
        }
        Subcommands::FromBin => {
            let hex = stdin::read_bytes(false)?;
            println!("0x{}", hex::encode(hex));
        }
        Subcommands::ToHexdata { input } => {
            let value = stdin::unwrap_line(input)?;
            let output = match value {
                s if s.starts_with('@') => hex::encode(std::env::var(&s[1..])?),
                s if s.starts_with('/') => hex::encode(fs::read(s)?),
                s => s.split(':').map(|s| s.trim_start_matches("0x").to_lowercase()).collect(),
            };
            println!("0x{output}");
        }
        Subcommands::ToCheckSumAddress { address } => {
            let value = stdin::unwrap_line(address)?;
            println!("{}", SimpleCast::to_checksum_address(&value));
        }
        Subcommands::ToUint256 { value } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_uint256(&value)?);
        }
        Subcommands::ToInt256 { value } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_int256(&value)?);
        }
        Subcommands::ToUnit { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_unit(&value, &unit)?);
        }
        Subcommands::FromWei { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::from_wei(&value, &unit)?);
        }
        Subcommands::ToWei { value, unit } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_wei(&value, &unit)?);
        }
        Subcommands::FromRlp { value } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::from_rlp(value)?);
        }
        Subcommands::ToRlp { value } => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_rlp(&value)?);
        }
        Subcommands::ToHex(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_base(&value, base_in, "hex")?);
        }
        Subcommands::ToDec(ToBaseArgs { value, base_in }) => {
            let value = stdin::unwrap_line(value)?;
            println!("{}", SimpleCast::to_base(&value, base_in, "dec")?);
        }
        Subcommands::ToBase { base: ToBaseArgs { value, base_in }, base_out } => {
            let (value, base_out) = stdin::unwrap2(value, base_out)?;
            println!("{}", SimpleCast::to_base(&value, base_in, &base_out)?);
        }
        Subcommands::ToBytes32 { bytes } => {
            let value = stdin::unwrap_line(bytes)?;
            println!("{}", SimpleCast::to_bytes32(&value)?);
        }
        Subcommands::FormatBytes32String { string } => {
            let value = stdin::unwrap_line(string)?;
            println!("{}", SimpleCast::format_bytes32_string(&value)?);
        }
        Subcommands::ParseBytes32String { bytes } => {
            let value = stdin::unwrap_line(bytes)?;
            println!("{}", SimpleCast::parse_bytes32_string(&value)?);
        }
        Subcommands::ParseBytes32Address { bytes } => {
            let value = stdin::unwrap_line(bytes)?;
            println!("{}", SimpleCast::parse_bytes32_address(&value)?);
        }

        // ABI encoding & decoding
        Subcommands::AbiDecode { sig, calldata, input } => {
            let tokens = SimpleCast::abi_decode(&sig, &calldata, input)?;
            let tokens = format_tokens(&tokens);
            tokens.for_each(|t| println!("{t}"));
        }
        Subcommands::AbiEncode { sig, args } => {
            println!("{}", SimpleCast::abi_encode(&sig, &args)?);
        }
        Subcommands::CalldataDecode { sig, calldata } => {
            let tokens = SimpleCast::calldata_decode(&sig, &calldata, true)?;
            let tokens = format_tokens(&tokens);
            tokens.for_each(|t| println!("{t}"));
        }
        Subcommands::CalldataEncode { sig, args } => {
            println!("{}", SimpleCast::calldata_encode(sig, &args)?);
        }
        Subcommands::Interface(cmd) => cmd.run().await?,
        Subcommands::Bind(cmd) => cmd.run().await?,
        Subcommands::PrettyCalldata { calldata, offline } => {
            let calldata = stdin::unwrap_line(calldata)?;
            println!("{}", pretty_calldata(&calldata, offline).await?);
        }
        Subcommands::Sig { sig, optimize } => {
            let sig = stdin::unwrap_line(sig)?;
            if optimize.is_none() {
                println!("{}", SimpleCast::get_selector(&sig, None)?.0);
            } else {
                println!("Starting to optimize signature...");
                let start_time = Instant::now();
                let (selector, signature) = SimpleCast::get_selector(&sig, optimize)?;
                let elapsed_time = start_time.elapsed();
                println!("Successfully generated in {} seconds", elapsed_time.as_secs());
                println!("Selector: {}", selector);
                println!("Optimized signature: {}", signature);
            }
        }

        // Blockchain & RPC queries
        Subcommands::AccessList(cmd) => cmd.run().await?,
        Subcommands::Age { block, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!(
                "{}",
                Cast::new(provider).age(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        Subcommands::Balance { block, who, ether, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let value = Cast::new(provider).balance(who, block).await?;
            if ether {
                println!("{}", SimpleCast::from_wei(&value.to_string(), "eth")?);
            } else {
                println!("{value}");
            }
        }
        Subcommands::BaseFee { block, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!(
                "{}",
                Cast::new(provider).base_fee(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        Subcommands::Block { block, full, field, json, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!(
                "{}",
                Cast::new(provider)
                    .block(block.unwrap_or(BlockId::Number(Latest)), full, field, json)
                    .await?
            );
        }
        Subcommands::BlockNumber { rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).block_number().await?);
        }
        Subcommands::Chain { rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).chain().await?);
        }
        Subcommands::ChainId { rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).chain_id().await?);
        }
        Subcommands::Client { rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", provider.client_version().await?);
        }
        Subcommands::Code { block, who, disassemble, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).code(who, block, disassemble).await?);
        }
        Subcommands::Codesize { block, who, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).codesize(who, block).await?);
        }
        Subcommands::ComputeAddress { address, nonce, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;

            let address: Address = stdin::unwrap_line(address)?.parse()?;
            let computed = Cast::new(&provider).compute_address(address, nonce).await?;
            println!("Computed Address: {}", SimpleCast::to_checksum_address(&computed));
        }
        Subcommands::Disassemble { bytecode } => {
            println!("{}", SimpleCast::disassemble(&bytecode)?);
        }
        Subcommands::FindBlock(cmd) => cmd.run().await?,
        Subcommands::GasPrice { rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).gas_price().await?);
        }
        Subcommands::Index { key_type, key, slot_number } => {
            println!("{}", SimpleCast::index(&key_type, &key, &slot_number)?);
        }
        Subcommands::Implementation { block, who, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).implementation(who, block).await?);
        }
        Subcommands::Admin { block, who, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).admin(who, block).await?);
        }
        Subcommands::Nonce { block, who, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!("{}", Cast::new(provider).nonce(who, block).await?);
        }
        Subcommands::Proof { address, slots, rpc, block } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let value = provider.get_proof(address, slots, block).await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        Subcommands::Rpc(cmd) => cmd.run().await?,
        Subcommands::Storage(cmd) => cmd.run().await?,

        // Calls & transactions
        Subcommands::Call(cmd) => cmd.run().await?,
        Subcommands::Estimate(cmd) => cmd.run().await?,
        Subcommands::PublishTx { raw_tx, cast_async, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            let cast = Cast::new(&provider);
            let pending_tx = cast.publish(raw_tx).await?;
            let tx_hash = *pending_tx;

            if cast_async {
                println!("{tx_hash:#x}");
            } else {
                let receipt =
                    pending_tx.await?.ok_or_else(|| eyre::eyre!("tx {tx_hash} not found"))?;
                println!("{}", serde_json::json!(receipt));
            }
        }
        Subcommands::Receipt { tx_hash, field, json, cast_async, confirmations, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;
            println!(
                "{}",
                Cast::new(provider)
                    .receipt(tx_hash, field, confirmations, cast_async, json)
                    .await?
            );
        }
        Subcommands::Run(cmd) => cmd.run().await?,
        Subcommands::SendTx(cmd) => cmd.run().await?,
        Subcommands::Tx { tx_hash, field, raw, json, rpc } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;

            // Can use either --raw or specify raw as a field
            let raw = raw || field.as_ref().is_some_and(|f| f == "raw");

            println!("{}", Cast::new(&provider).transaction(tx_hash, field, raw, json).await?)
        }

        // 4Byte
        Subcommands::FourByte { selector } => {
            let selector = stdin::unwrap_line(selector)?;
            let sigs = decode_function_selector(&selector).await?;
            if sigs.is_empty() {
                eyre::bail!("No matching function signatures found for selector `{selector}`");
            }
            for sig in sigs {
                println!("{sig}");
            }
        }
        Subcommands::FourByteDecode { calldata } => {
            let calldata = stdin::unwrap_line(calldata)?;
            let sigs = decode_calldata(&calldata).await?;
            sigs.iter().enumerate().for_each(|(i, sig)| println!("{}) \"{sig}\"", i + 1));

            let sig = match sigs.len() {
                0 => eyre::bail!("No signatures found"),
                1 => sigs.get(0).unwrap(),
                _ => {
                    let i: usize = prompt!("Select a function signature by number: ")?;
                    sigs.get(i - 1).ok_or_else(|| eyre::eyre!("Invalid signature index"))?
                }
            };

            let tokens = SimpleCast::calldata_decode(sig, &calldata, true)?;
            for token in format_tokens(&tokens) {
                println!("{token}");
            }
        }
        Subcommands::FourByteEvent { topic } => {
            let topic = stdin::unwrap_line(topic)?;
            let sigs = decode_event_topic(&topic).await?;
            if sigs.is_empty() {
                eyre::bail!("No matching event signatures found for topic `{topic}`");
            }
            for sig in sigs {
                println!("{sig}");
            }
        }
        Subcommands::UploadSignature { signatures } => {
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
        Subcommands::Namehash { name } => {
            let name = stdin::unwrap_line(name)?;
            println!("{}", SimpleCast::namehash(&name)?);
        }
        Subcommands::LookupAddress { who, rpc, verify } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;

            let who = stdin::unwrap_line(who)?;
            let name = provider.lookup_address(who).await?;
            if verify {
                let address = provider.resolve_name(&name).await?;
                eyre::ensure!(
                    address == who,
                    "Forward lookup verification failed: got `{name:?}`, expected `{who:?}`"
                );
            }
            println!("{name}");
        }
        Subcommands::ResolveName { who, rpc, verify } => {
            let config = Config::from(&rpc);
            let provider = utils::get_provider(&config)?;

            let who = stdin::unwrap_line(who)?;
            let address = provider.resolve_name(&who).await?;
            if verify {
                let name = provider.lookup_address(address).await?;
                assert_eq!(
                    name, who,
                    "forward lookup verification failed. got {name}, expected {who}"
                );
            }
            println!("{}", SimpleCast::to_checksum_address(&address));
        }

        // Misc
        Subcommands::Keccak { data } => {
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
        Subcommands::SigEvent { event_string } => {
            let event_string = stdin::unwrap_line(event_string)?;
            let parsed_event = get_event(&event_string)?;
            println!("{:?}", parsed_event.signature());
        }
        Subcommands::LeftShift { value, bits, base_in, base_out } => {
            println!("{}", SimpleCast::left_shift(&value, &bits, base_in, &base_out)?);
        }
        Subcommands::RightShift { value, bits, base_in, base_out } => {
            println!("{}", SimpleCast::right_shift(&value, &bits, base_in, &base_out)?);
        }
        Subcommands::EtherscanSource { address, directory, etherscan } => {
            let config = Config::from(&etherscan);
            let chain = config.chain_id.unwrap_or_default();
            let api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();
            let chain = chain.named()?;
            match directory {
                Some(dir) => {
                    SimpleCast::expand_etherscan_source_to_directory(chain, address, api_key, dir)
                        .await?
                }
                None => {
                    println!("{}", SimpleCast::etherscan_source(chain, address, api_key).await?);
                }
            }
        }
        Subcommands::Create2(cmd) => {
            cmd.run()?;
        }
        Subcommands::Wallet { command } => command.run().await?,
        Subcommands::Completions { shell } => {
            generate(shell, &mut Opts::command(), "cast", &mut std::io::stdout())
        }
        Subcommands::GenerateFigSpec => clap_complete::generate(
            clap_complete_fig::Fig,
            &mut Opts::command(),
            "cast",
            &mut std::io::stdout(),
        ),
        Subcommands::Logs(cmd) => cmd.run().await?,
    };
    Ok(())
}
