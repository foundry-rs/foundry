use cast::{Cast, SimpleCast, TxBuilder};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use ethers::{
    abi::HumanReadableParser,
    core::types::{BlockId, BlockNumber::Latest, H256},
    providers::Middleware,
    types::Address,
    utils::keccak256,
};
use foundry_cli::{
    cmd::Cmd,
    handler,
    opts::cast::{Opts, Subcommands, ToBaseArgs},
    prompt, stdin, utils,
    utils::try_consume_config_rpc_url,
};
use foundry_common::{
    abi::{format_tokens, get_event},
    fs,
    selectors::{
        decode_calldata, decode_event_topic, decode_function_selector, import_selectors,
        parse_signatures, pretty_calldata, ParsedSignatures, SelectorImportData,
    },
    try_get_http_provider,
};
use foundry_config::{Chain, Config};
use rustc_hex::ToHex;

#[tokio::main]
async fn main() -> eyre::Result<()> {
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
                s if s.starts_with('@') => {
                    let var = std::env::var(&s[1..])?;
                    var.as_bytes().to_hex()
                }
                s if s.starts_with('/') => {
                    let input = fs::read(s)?;
                    input.to_hex()
                }
                s => {
                    let mut output = String::new();
                    for s in s.split(':') {
                        output.push_str(&s.trim_start_matches("0x").to_lowercase())
                    }
                    output
                }
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
            let tokens = SimpleCast::abi_decode(&sig, &calldata, true)?;
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
        Subcommands::Sig { sig } => {
            let sig = stdin::unwrap_line(sig)?;
            let selector = HumanReadableParser::parse_function(&sig)?.short_signature();
            println!("0x{}", hex::encode(selector));
        }

        // Blockchain & RPC queries
        Subcommands::AccessList { eth, address, sig, args, block, to_json } => {
            let config = Config::from(&eth);
            let provider = try_get_http_provider(config.get_rpc_url_or_localhost_http()?)?;

            let chain: Chain = if let Some(chain) = eth.chain {
                chain
            } else {
                provider.get_chainid().await?.into()
            };

            let mut builder =
                TxBuilder::new(&provider, config.sender, Some(address), chain, false).await?;
            builder.set_args(&sig, args).await?;
            let builder_output = builder.peek();

            println!("{}", Cast::new(&provider).access_list(builder_output, block, to_json).await?);
        }
        Subcommands::Age { block, rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!(
                "{}",
                Cast::new(provider).age(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        Subcommands::Balance { block, who, rpc_url, to_ether } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            let value = Cast::new(provider).balance(who, block).await?;
            if to_ether {
                println!("{}", SimpleCast::from_wei(&value.to_string(), "eth")?);
            } else {
                println!("{value}");
            }
        }
        Subcommands::BaseFee { block, rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!(
                "{}",
                Cast::new(provider).base_fee(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        Subcommands::Block { rpc_url, block, full, field, to_json } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(provider).block(block, full, field, to_json).await?);
        }
        Subcommands::BlockNumber { rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(provider).block_number().await?);
        }
        Subcommands::Chain { rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(provider).chain().await?);
        }
        Subcommands::ChainId { rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(provider).chain_id().await?);
        }
        Subcommands::Client { rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", provider.client_version().await?);
        }
        Subcommands::Code { block, who, rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(provider).code(who, block).await?);
        }
        Subcommands::ComputeAddress { address, nonce, rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;

            let address: Address = stdin::unwrap_line(address)?.parse()?;
            let computed = Cast::new(&provider).compute_address(address, nonce).await?;
            println!("Computed Address: {}", SimpleCast::to_checksum_address(&computed));
        }
        Subcommands::FindBlock(cmd) => cmd.run().await?,
        Subcommands::GasPrice { rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(provider).gas_price().await?);
        }
        Subcommands::Index { key_type, key, slot_number } => {
            let encoded = SimpleCast::index(&key_type, &key, &slot_number)?;
            println!("{encoded}");
        }
        Subcommands::Implementation { block, who, rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(provider).implementation(who, block).await?);
        }
        Subcommands::Admin { block, who, rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(provider).admin(who, block).await?);
        }
        Subcommands::Nonce { block, who, rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(provider).nonce(who, block).await?);
        }
        Subcommands::Proof { address, slots, rpc_url, block } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            let value = provider.get_proof(address, slots, block).await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        Subcommands::Rpc(cmd) => cmd.run().await?,
        Subcommands::Storage(cmd) => cmd.run().await?,

        // Calls & transactions
        Subcommands::Call(cmd) => cmd.run().await?,
        Subcommands::Estimate(cmd) => cmd.run().await?,
        Subcommands::PublishTx { eth, raw_tx, cast_async } => {
            let config = Config::from(&eth);
            let provider = try_get_http_provider(config.get_rpc_url_or_localhost_http()?)?;
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
        Subcommands::Receipt { tx_hash, field, to_json, rpc_url, cast_async, confirmations } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!(
                "{}",
                Cast::new(provider)
                    .receipt(tx_hash, field, confirmations, cast_async, to_json)
                    .await?
            );
        }
        Subcommands::Run(cmd) => cmd.run().await?,
        Subcommands::SendTx(cmd) => cmd.run().await?,
        Subcommands::Tx { tx_hash, field, to_json, rpc_url } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;
            println!("{}", Cast::new(&provider).transaction(tx_hash, field, to_json).await?)
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

            let tokens = SimpleCast::abi_decode(sig, &calldata, true)?;
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
        Subcommands::LookupAddress { who, rpc_url, verify } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;

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
        Subcommands::ResolveName { who, rpc_url, verify } => {
            let rpc_url = try_consume_config_rpc_url(rpc_url)?;
            let provider = try_get_http_provider(rpc_url)?;

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
        Subcommands::EtherscanSource { chain, address, directory, etherscan_api_key } => {
            let api_key = match etherscan_api_key {
                Some(inner) => inner,
                _ => {
                    if let Some(etherscan_api_key) = Config::load().etherscan_api_key {
                        etherscan_api_key
                    } else {
                        eyre::bail!("No Etherscan API Key is set. Consider using the ETHERSCAN_API_KEY env var, or setting the -e CLI argument or etherscan-api-key in foundry.toml")
                    }
                }
            };
            match directory {
                Some(dir) => {
                    SimpleCast::expand_etherscan_source_to_directory(
                        chain.inner,
                        address,
                        api_key,
                        dir,
                    )
                    .await?
                }
                None => {
                    println!(
                        "{}",
                        SimpleCast::etherscan_source(chain.inner, address, api_key).await?
                    );
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
    };
    Ok(())
}
