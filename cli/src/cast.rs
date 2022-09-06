use cast::{Cast, SimpleCast, TxBuilder};
use clap::{IntoApp, Parser};
use clap_complete::generate;
use ethers::{
    abi::HumanReadableParser,
    core::types::{BlockId, BlockNumber::Latest, H256},
    providers::Middleware,
    types::{Address, U256},
};

use foundry_cli::{
    cmd::Cmd,
    handler,
    opts::cast::{Opts, Subcommands},
    utils,
    utils::consume_config_rpc_url,
};
use foundry_common::{fs, get_http_provider};
use foundry_config::{Chain, Config};
use foundry_utils::{
    format_tokens,
    selectors::{
        decode_calldata, decode_event_topic, decode_function_selector, import_selectors,
        parse_signatures, pretty_calldata, ParsedSignatures, SelectorImportData,
    },
};
use rustc_hex::ToHex;
use std::{
    io::{self, Read, Write},
    str::FromStr,
};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    utils::load_dotenv();
    handler::install()?;
    utils::subscriber();
    utils::enable_paint();

    let opts = Opts::parse();
    match opts.sub {
        Subcommands::MaxInt => {
            println!("{}", SimpleCast::max_int()?);
        }
        Subcommands::MinInt => {
            println!("{}", SimpleCast::min_int()?);
        }
        Subcommands::MaxUint => {
            println!("{}", SimpleCast::max_uint()?);
        }
        Subcommands::AddressZero => {
            println!("{:?}", Address::zero());
        }
        Subcommands::HashZero => {
            println!("{:?}", H256::zero());
        }
        Subcommands::FromUtf8 { text } => {
            let val = unwrap_or_stdin(text)?;
            println!("{}", SimpleCast::from_utf8(&val));
        }
        Subcommands::ToHex { decimal } => {
            let val = unwrap_or_stdin(decimal)?;
            println!("{}", SimpleCast::hex(U256::from_dec_str(&val)?));
        }
        Subcommands::ConcatHex { data } => {
            println!("{}", SimpleCast::concat_hex(data))
        }
        Subcommands::FromBin {} => {
            let hex: String = io::stdin()
                .bytes()
                .map(|x| format!("{:02x}", x.expect("invalid binary data")))
                .collect();
            println!("0x{hex}");
        }

        Subcommands::ToHexdata { input } => {
            let val = unwrap_or_stdin(input)?;
            let output = match val {
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
            let val = unwrap_or_stdin(address)?;
            println!("{}", SimpleCast::checksum_address(&val)?);
        }
        Subcommands::ToAscii { hexdata } => {
            let val = unwrap_or_stdin(hexdata)?;
            println!("{}", SimpleCast::ascii(&val)?);
        }
        Subcommands::FromFix { decimals, value } => {
            let val = unwrap_or_stdin(value)?;
            println!("{}", SimpleCast::from_fix(unwrap_or_stdin(decimals)? as u32, &val)?);
        }
        Subcommands::ToBytes32 { bytes } => {
            let val = unwrap_or_stdin(bytes)?;
            println!("{}", SimpleCast::bytes32(&val)?);
        }
        Subcommands::ToDec { hexvalue } => {
            let val = unwrap_or_stdin(hexvalue)?;
            println!("{}", SimpleCast::to_dec(&val)?);
        }
        Subcommands::ToFix { decimals, value } => {
            let val = unwrap_or_stdin(value)?;
            println!(
                "{}",
                SimpleCast::to_fix(unwrap_or_stdin(decimals)?, U256::from_dec_str(&val)?)?
            );
        }
        Subcommands::ToUint256 { value } => {
            let val = unwrap_or_stdin(value)?;
            println!("{}", SimpleCast::to_uint256(&val)?);
        }
        Subcommands::ToInt256 { value } => {
            let val = unwrap_or_stdin(value)?;
            println!("{}", SimpleCast::to_int256(&val)?);
        }
        Subcommands::LeftShift { value, bits, base_in, base_out } => {
            println!(
                "{}",
                format_uint(
                    SimpleCast::left_shift(&value, &bits, det_base_in(&value, base_in)?)?,
                    det_base_out(&base_out)?
                )?
            );
        }
        Subcommands::RightShift { value, bits, base_in, base_out } => {
            println!(
                "{}",
                format_uint(
                    SimpleCast::right_shift(&value, &bits, det_base_in(&value, base_in)?)?,
                    det_base_out(&base_out)?
                )?
            );
        }
        Subcommands::ToUnit { value, unit } => {
            let val = unwrap_or_stdin(value)?;
            println!("{}", SimpleCast::to_unit(val, unit)?);
        }
        Subcommands::ToWei { value, unit } => {
            let val = unwrap_or_stdin(value)?;
            println!(
                "{}",
                SimpleCast::to_wei(
                    val.parse::<f64>()?,
                    unit.unwrap_or_else(|| String::from("eth"))
                )?
            );
        }
        Subcommands::FromWei { value, unit } => {
            let val = unwrap_or_stdin(value)?;
            println!(
                "{}",
                SimpleCast::from_wei(
                    U256::from_dec_str(&val)?,
                    unit.unwrap_or_else(|| String::from("eth"))
                )?
            );
        }
        Subcommands::ToRlp { value } => {
            let val = unwrap_or_stdin(value)?;
            println!("{}", SimpleCast::to_rlp(&val)?);
        }
        Subcommands::FromRlp { value } => {
            let val = unwrap_or_stdin(value)?;
            println!("{}", SimpleCast::from_rlp(val)?);
        }
        Subcommands::AccessList { eth, address, sig, args, block, to_json } => {
            let config = Config::from(&eth);
            let provider = get_http_provider(
                config.eth_rpc_url.unwrap_or_else(|| "http://localhost:8545".to_string()),
            );

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
        Subcommands::Block { rpc_url, block, full, field, to_json } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            println!("{}", Cast::new(provider).block(block, full, field, to_json).await?);
        }
        Subcommands::BlockNumber { rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            println!("{}", Cast::new(provider).block_number().await?);
        }
        Subcommands::Call(cmd) => cmd.run().await?,
        Subcommands::Calldata { sig, args } => {
            println!("{}", SimpleCast::calldata(sig, &args)?);
        }
        Subcommands::Chain { rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            println!("{}", Cast::new(provider).chain().await?);
        }
        Subcommands::ChainId { rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);

            let provider = get_http_provider(rpc_url);
            println!("{}", Cast::new(provider).chain_id().await?);
        }
        Subcommands::Client { rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);

            let provider = get_http_provider(rpc_url);
            println!("{}", provider.client_version().await?);
        }
        Subcommands::ComputeAddress { rpc_url, address, nonce } => {
            let rpc_url = consume_config_rpc_url(rpc_url);

            let pubkey = Address::from_str(&address).expect("invalid pubkey provided");
            let provider = get_http_provider(rpc_url);
            let addr = Cast::new(&provider).compute_address(pubkey, nonce).await?;
            println!("Computed Address: {}", SimpleCast::checksum_address(&addr)?);
        }
        Subcommands::Code { block, who, rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            println!("{}", Cast::new(provider).code(who, block).await?);
        }
        Subcommands::Namehash { name } => {
            println!("{}", SimpleCast::namehash(&name)?);
        }
        Subcommands::Tx { rpc_url, hash, field, to_json } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            println!("{}", Cast::new(&provider).transaction(hash, field, to_json).await?)
        }
        Subcommands::SendTx(cmd) => cmd.run().await?,
        Subcommands::PublishTx { eth, raw_tx, cast_async } => {
            let config = Config::from(&eth);
            let provider = get_http_provider(
                config.eth_rpc_url.unwrap_or_else(|| "http://localhost:8545".to_string()),
            );
            let cast = Cast::new(&provider);
            let pending_tx = cast.publish(raw_tx).await?;
            let tx_hash = *pending_tx;

            if cast_async {
                println!("{:?}", pending_tx);
            } else {
                let receipt =
                    pending_tx.await?.ok_or_else(|| eyre::eyre!("tx {tx_hash} not found"))?;
                println!("{}", serde_json::json!(receipt));
            }
        }
        Subcommands::CalldataDecode { sig, calldata } => {
            let tokens = SimpleCast::abi_decode(&sig, &calldata, true)?;
            let tokens = format_tokens(&tokens);
            tokens.for_each(|t| println!("{t}"));
        }
        Subcommands::AbiDecode { sig, calldata, input } => {
            let tokens = SimpleCast::abi_decode(&sig, &calldata, input)?;
            let tokens = format_tokens(&tokens);
            tokens.for_each(|t| println!("{t}"));
        }
        Subcommands::AbiEncode { sig, args } => {
            println!("{}", SimpleCast::abi_encode(&sig, &args)?);
        }
        Subcommands::Index { key_type, key, slot_number } => {
            let encoded = SimpleCast::index(&key_type, &key, &slot_number)?;
            println!("{encoded}");
        }
        Subcommands::FourByte { selector } => {
            let sigs = decode_function_selector(&selector).await?;
            sigs.iter().for_each(|sig| println!("{}", sig));
        }
        Subcommands::FourByteDecode { calldata } => {
            let calldata = unwrap_or_stdin(calldata)?;
            let sigs = decode_calldata(&calldata).await?;
            sigs.iter().enumerate().for_each(|(i, sig)| println!("{}) \"{}\"", i + 1, sig));

            let sig = match sigs.len() {
                0 => Err(eyre::eyre!("No signatures found")),
                1 => Ok(sigs.get(0).unwrap()),
                _ => {
                    print!("Select a function signature by number: ");
                    io::stdout().flush()?;
                    let mut input = String::new();
                    io::stdin().read_line(&mut input)?;
                    let i: usize = input.trim().parse()?;
                    Ok(sigs.get(i - 1).expect("Invalid signature index"))
                }
            }?;

            let tokens = SimpleCast::abi_decode(sig, &calldata, true)?;
            let tokens = format_tokens(&tokens);

            tokens.for_each(|t| println!("{t}"));
        }
        Subcommands::FourByteEvent { topic } => {
            let sigs = decode_event_topic(&topic).await?;
            sigs.iter().for_each(|sig| println!("{}", sig));
        }

        Subcommands::UploadSignature { signatures } => {
            let ParsedSignatures { signatures, abis } = parse_signatures(signatures);
            if !abis.is_empty() {
                import_selectors(SelectorImportData::Abi(abis)).await?.describe();
            }
            if !signatures.is_empty() {
                import_selectors(SelectorImportData::Raw(signatures)).await?.describe();
            }
        }

        Subcommands::PrettyCalldata { calldata, offline } => {
            if !calldata.starts_with("0x") {
                eprintln!("Expected calldata hex string, received \"{calldata}\"");
                std::process::exit(0)
            }
            let pretty_data = pretty_calldata(&calldata, offline).await?;
            println!("{pretty_data}");
        }
        Subcommands::Age { block, rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            println!(
                "{}",
                Cast::new(provider).age(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        Subcommands::Balance { block, who, rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            println!("{}", Cast::new(provider).balance(who, block).await?);
        }
        Subcommands::BaseFee { block, rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);

            let provider = get_http_provider(rpc_url);
            println!(
                "{}",
                Cast::new(provider).base_fee(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        Subcommands::GasPrice { rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            println!("{}", Cast::new(provider).gas_price().await?);
        }
        Subcommands::Keccak { data } => {
            println!("{}", SimpleCast::keccak(&data)?);
        }

        Subcommands::Interface(cmd) => cmd.run()?.await?,
        Subcommands::ResolveName { who, rpc_url, verify } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            let who = unwrap_or_stdin(who)?;
            let address = provider.resolve_name(&who).await?;
            if verify {
                let name = provider.lookup_address(address).await?;
                assert_eq!(
                    name, who,
                    "forward lookup verification failed. got {}, expected {}",
                    name, who
                );
            }
            println!("{}", SimpleCast::checksum_address(&address)?);
        }
        Subcommands::LookupAddress { who, rpc_url, verify } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            let who = unwrap_or_stdin(who)?;
            let name = provider.lookup_address(who).await?;
            if verify {
                let address = provider.resolve_name(&name).await?;
                assert_eq!(
                    address, who,
                    "forward lookup verification failed. got {}, expected {}",
                    name, who
                );
            }
            println!("{name}");
        }
        Subcommands::Storage { address, slot, rpc_url, block } => {
            let rpc_url = consume_config_rpc_url(rpc_url);

            let provider = get_http_provider(rpc_url);
            let value = provider.get_storage_at(address, slot, block).await?;
            println!("{:?}", value);
        }
        Subcommands::Proof { address, slots, rpc_url, block } => {
            let rpc_url = consume_config_rpc_url(rpc_url);

            let provider = get_http_provider(rpc_url);
            let value = provider.get_proof(address, slots, block).await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        Subcommands::Receipt { hash, field, to_json, rpc_url, cast_async, confirmations } => {
            let rpc_url = consume_config_rpc_url(rpc_url);
            let provider = get_http_provider(rpc_url);
            println!(
                "{}",
                Cast::new(provider)
                    .receipt(hash, field, confirmations, cast_async, to_json)
                    .await?
            );
        }
        Subcommands::Nonce { block, who, rpc_url } => {
            let rpc_url = consume_config_rpc_url(rpc_url);

            let provider = get_http_provider(rpc_url);
            println!("{}", Cast::new(provider).nonce(who, block).await?);
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
        Subcommands::Sig { sig } => {
            let selector = HumanReadableParser::parse_function(&sig)?.short_signature();
            println!("0x{}", hex::encode(selector));
        }
        Subcommands::FindBlock(cmd) => cmd.run()?.await?,
        Subcommands::Estimate(cmd) => cmd.run().await?,
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
        Subcommands::Run(cmd) => cmd.run()?,
        Subcommands::Rpc(cmd) => cmd.run()?.await?,
        Subcommands::FormatBytes32String { string } => {
            let val = unwrap_or_stdin(string)?;
            println!("{}", SimpleCast::format_bytes32_string(&val)?);
        }
        Subcommands::ParseBytes32String { bytes } => {
            let val = unwrap_or_stdin(bytes)?;
            println!("{}", SimpleCast::parse_bytes32_string(&val)?);
        }
    };
    Ok(())
}

fn unwrap_or_stdin<T>(what: Option<T>) -> eyre::Result<T>
where
    T: FromStr + Send + Sync,
    T::Err: Send + Sync + std::error::Error + 'static,
{
    Ok(match what {
        Some(what) => what,
        None => {
            let input = std::io::stdin();
            let mut what = String::new();
            input.read_line(&mut what)?;
            T::from_str(&what.replace('\n', ""))?
        }
    })
}

fn det_base_in(value: &str, base_in: Option<String>) -> eyre::Result<u32> {
    match base_in {
        Some(base_in) => match base_in.as_str() {
            "10" | "dec" => Ok(10),
            "16" | "hex" => Ok(16),
            _ => eyre::bail!("Unknown input base: {base_in}"),
        },
        None if value.starts_with("0x") => Ok(16),
        None => match U256::from_str_radix(value, 10) {
            Ok(_) => {
                eyre::bail!("Could not autodetect input base: input could be decimal or hexadecimal. Please prepend with 0x if the input is hexadecimal, or specify a --base-in parameter.");
            }
            Err(_) => {
                U256::from_str_radix(value, 16).expect("Could not autodetect input base.");
                Ok(16)
            }
        },
    }
}

fn det_base_out(base_out: &str) -> eyre::Result<u32> {
    match base_out {
        "10" | "dec" => Ok(10),
        "16" | "hex" => Ok(16),
        _ => eyre::bail!("Provided base is not a valid."),
    }
}

fn format_uint(val: U256, base_out: u32) -> eyre::Result<String> {
    match base_out {
        10 => Ok(val.to_string()),
        16 => Ok(format!("0x{:x}", val)),
        _ => Err(eyre::eyre!("Unknown output base: {base_out}")),
    }
}
