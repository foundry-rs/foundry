pub mod cmd;

mod utils;

use cast::{Cast, SimpleCast};

mod opts;
use cast::InterfacePath;
use ethers::{
    contract::BaseContract,
    core::{
        abi::parse_abi,
        rand::thread_rng,
        types::{BlockId, BlockNumber::Latest},
    },
    providers::{Middleware, Provider},
    signers::{LocalWallet, Signer},
    types::{Address, Chain, NameOrAddress, Signature, U256, U64},
};
use opts::{
    cast::{Opts, Subcommands, WalletSubcommands},
    EthereumOpts, WalletType,
};
use rayon::prelude::*;
use regex::RegexSet;
use rustc_hex::ToHex;
use std::{
    convert::TryFrom,
    io::{self, Write},
    path::Path,
    str::FromStr,
    time::Instant,
};

use clap::{IntoApp, Parser};
use clap_complete::generate;

use crate::utils::read_secret;
use eyre::WrapErr;
use futures::join;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

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
        Subcommands::FromUtf8 { text } => {
            let val = unwrap_or_stdin(text)?;
            println!("{}", SimpleCast::from_utf8(&val));
        }
        Subcommands::ToHex { decimal } => {
            let val = unwrap_or_stdin(decimal)?;
            println!("{}", SimpleCast::hex(U256::from_dec_str(&val)?));
        }
        Subcommands::ToHexdata { input } => {
            let val = unwrap_or_stdin(input)?;
            let output = match val {
                s if s.starts_with('@') => {
                    let var = std::env::var(&s[1..])?;
                    var.as_bytes().to_hex()
                }
                s if s.starts_with('/') => {
                    let input = std::fs::read(s)?;
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
            println!("0x{}", output);
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
        Subcommands::ToUnit { value, unit } => {
            let val = unwrap_or_stdin(value)?;
            println!("{}", SimpleCast::to_unit(val, unit.unwrap_or_else(|| String::from("wei")))?);
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
        Subcommands::Block { rpc_url, block, full, field, to_json } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Cast::new(provider).block(block, full, field, to_json).await?);
        }
        Subcommands::BlockNumber { rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Cast::new(provider).block_number().await?);
        }
        Subcommands::Call { eth, address, sig, args, block } => {
            let provider = Provider::try_from(eth.rpc_url()?)?;
            println!(
                "{}",
                Cast::new(provider)
                    .call(
                        eth.from.unwrap_or(Address::zero()),
                        address,
                        (&sig, args),
                        eth.chain,
                        eth.etherscan_api_key,
                        block
                    )
                    .await?
            );
        }
        Subcommands::Calldata { sig, args } => {
            println!("{}", SimpleCast::calldata(sig, &args)?);
        }
        Subcommands::Chain { rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Cast::new(provider).chain().await?);
        }
        Subcommands::ChainId { rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Cast::new(provider).chain_id().await?);
        }
        Subcommands::Client { rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", provider.client_version().await?);
        }
        Subcommands::Code { block, who, rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Cast::new(provider).code(who, block).await?);
        }
        Subcommands::Namehash { name } => {
            println!("{}", SimpleCast::namehash(&name)?);
        }
        Subcommands::Tx { rpc_url, hash, field, to_json } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Cast::new(&provider).transaction(hash, field, to_json).await?)
        }
        Subcommands::SendTx {
            eth,
            to,
            sig,
            cast_async,
            args,
            gas,
            gas_price,
            value,
            nonce,
            legacy,
            confirmations,
            to_json,
        } => {
            let provider = Provider::try_from(eth.rpc_url()?)?;
            let chain_id = Cast::new(&provider).chain_id().await?;
            let sig = sig.unwrap_or_default();

            if let Some(signer) = eth.signer_with(chain_id, provider.clone()).await? {
                match signer {
                    WalletType::Ledger(signer) => {
                        cast_send(
                            &signer,
                            signer.address(),
                            to,
                            (sig, args),
                            gas,
                            gas_price,
                            value,
                            nonce,
                            eth.chain,
                            eth.etherscan_api_key,
                            cast_async,
                            legacy,
                            confirmations,
                            to_json,
                        )
                        .await?;
                    }
                    WalletType::Local(signer) => {
                        cast_send(
                            &signer,
                            signer.address(),
                            to,
                            (sig, args),
                            gas,
                            gas_price,
                            value,
                            nonce,
                            eth.chain,
                            eth.etherscan_api_key,
                            cast_async,
                            legacy,
                            confirmations,
                            to_json,
                        )
                        .await?;
                    }
                    WalletType::Trezor(signer) => {
                        cast_send(
                            &signer,
                            signer.address(),
                            to,
                            (sig, args),
                            gas,
                            gas_price,
                            value,
                            nonce,
                            eth.chain,
                            eth.etherscan_api_key,
                            cast_async,
                            legacy,
                            confirmations,
                            to_json,
                        )
                        .await?;
                    }
                }
            } else {
                let from = eth.from.expect("No ETH_FROM or signer specified");
                cast_send(
                    provider,
                    from,
                    to,
                    (sig, args),
                    gas,
                    gas_price,
                    value,
                    nonce,
                    eth.chain,
                    eth.etherscan_api_key,
                    cast_async,
                    legacy,
                    confirmations,
                    to_json,
                )
                .await?;
            }
        }
        Subcommands::PublishTx { eth, raw_tx, cast_async } => {
            let provider = Provider::try_from(eth.rpc_url()?)?;
            let cast = Cast::new(&provider);
            let pending_tx = cast.publish(raw_tx).await?;
            let tx_hash = *pending_tx;

            if cast_async {
                println!("{:?}", pending_tx);
            } else {
                let receipt =
                    pending_tx.await?.ok_or_else(|| eyre::eyre!("tx {} not found", tx_hash))?;
                println!("{}", serde_json::json!(receipt));
            }
        }
        Subcommands::Estimate { eth, to, sig, args, value } => {
            let provider = Provider::try_from(eth.rpc_url()?)?;
            let cast = Cast::new(&provider);
            let from = eth.sender().await;
            let gas = cast
                .estimate(
                    from,
                    to,
                    Some((sig.as_str(), args)),
                    value,
                    eth.chain,
                    eth.etherscan_api_key,
                )
                .await?;
            println!("{}", gas);
        }
        Subcommands::CalldataDecode { sig, calldata } => {
            let tokens = SimpleCast::abi_decode(&sig, &calldata, true)?;
            let tokens = foundry_utils::format_tokens(&tokens);
            tokens.for_each(|t| println!("{}", t));
        }
        Subcommands::AbiDecode { sig, calldata, input } => {
            let tokens = SimpleCast::abi_decode(&sig, &calldata, input)?;
            let tokens = foundry_utils::format_tokens(&tokens);
            tokens.for_each(|t| println!("{}", t));
        }
        Subcommands::AbiEncode { sig, args } => {
            println!("{}", SimpleCast::abi_encode(&sig, &args)?);
        }
        Subcommands::Index { from_type, to_type, from_value, slot_number } => {
            let encoded = SimpleCast::index(&from_type, &to_type, &from_value, &slot_number)?;
            println!("{}", encoded);
        }
        Subcommands::FourByte { selector } => {
            let sigs = foundry_utils::fourbyte(&selector).await?;
            sigs.iter().for_each(|sig| println!("{}", sig.0));
        }
        Subcommands::FourByteDecode { calldata, id } => {
            let sigs = foundry_utils::fourbyte_possible_sigs(&calldata, id).await?;
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
            let tokens = foundry_utils::format_tokens(&tokens);

            tokens.for_each(|t| println!("{}", t));
        }
        Subcommands::FourByteEvent { topic } => {
            let sigs = foundry_utils::fourbyte_event(&topic).await?;
            sigs.iter().for_each(|sig| println!("{}", sig.0));
        }

        Subcommands::PrettyCalldata { calldata, offline } => {
            if !calldata.starts_with("0x") {
                eprintln!("Expected calldata hex string, received \"{}\"", calldata);
                std::process::exit(0)
            }
            let pretty_data = foundry_utils::pretty_calldata(&calldata, offline).await?;
            println!("{}", pretty_data);
        }
        Subcommands::Age { block, rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!(
                "{}",
                Cast::new(provider).age(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        Subcommands::Balance { block, who, rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Cast::new(provider).balance(who, block).await?);
        }
        Subcommands::BaseFee { block, rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!(
                "{}",
                Cast::new(provider).base_fee(block.unwrap_or(BlockId::Number(Latest))).await?
            );
        }
        Subcommands::GasPrice { rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Cast::new(provider).gas_price().await?);
        }
        Subcommands::Keccak { data } => {
            println!("{}", SimpleCast::keccak(&data)?);
        }

        Subcommands::Interface {
            path_or_address,
            pragma,
            chain,
            output_location,
            etherscan_api_key,
        } => {
            let interfaces = if Path::new(&path_or_address).exists() {
                SimpleCast::generate_interface(InterfacePath::Local(path_or_address)).await?
            } else {
                let api_key = match etherscan_api_key {
                    Some(inner) => inner,
                    _ => eyre::bail!("No Etherscan API Key is set. Consider using the ETHERSCAN_API_KEY env var, or the -e CLI argument.")
                };
                SimpleCast::generate_interface(InterfacePath::Etherscan {
                    chain: chain.inner,
                    api_key,
                    address: path_or_address
                        .parse::<Address>()
                        .wrap_err("Invalid address provided. Did you make a typo?")?,
                })
                .await?
            };

            // put it all together
            let pragma = format!("pragma solidity {};", pragma);
            let interfaces = interfaces
                .iter()
                .map(|iface| iface.source.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let res = format!("{}\n\n{}", pragma, interfaces);

            // print or write to file
            match output_location {
                Some(loc) => {
                    std::fs::create_dir_all(&loc.parent().unwrap())?;
                    std::fs::write(&loc, res)?;
                    println!("Saved interface at {}", loc.display());
                }
                None => {
                    println!("{}", res);
                }
            }
        }
        Subcommands::ResolveName { who, rpc_url, verify } => {
            let provider = Provider::try_from(rpc_url)?;
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
            println!("{:?}", address);
        }
        Subcommands::LookupAddress { who, rpc_url, verify } => {
            let provider = Provider::try_from(rpc_url)?;
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
            println!("{}", name);
        }
        Subcommands::Storage { address, slot, rpc_url, block } => {
            let provider = Provider::try_from(rpc_url)?;
            let value = provider.get_storage_at(address, slot, block).await?;
            println!("{:?}", value);
        }
        Subcommands::Proof { address, slots, rpc_url, block } => {
            let provider = Provider::try_from(rpc_url)?;
            let value = provider.get_proof(address, slots, block).await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        Subcommands::Receipt { hash, field, to_json, rpc_url, cast_async, confirmations } => {
            let provider = Provider::try_from(rpc_url)?;
            println!(
                "{}",
                Cast::new(provider)
                    .receipt(hash, field, confirmations, cast_async, to_json)
                    .await?
            );
        }
        Subcommands::Nonce { block, who, rpc_url } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Cast::new(provider).nonce(who, block).await?);
        }
        Subcommands::EtherscanSource { chain, address, etherscan_api_key } => {
            println!(
                "{}",
                SimpleCast::etherscan_source(chain.inner, address, etherscan_api_key).await?
            );
        }
        Subcommands::Sig { sig } => {
            let contract = BaseContract::from(parse_abi(&[&sig]).unwrap());
            let selector = contract.abi().functions().last().unwrap().short_signature();
            println!("0x{}", hex::encode(selector));
        }
        Subcommands::FindBlock { timestamp, rpc_url } => {
            let ts_target = U256::from(timestamp);
            let provider = Provider::try_from(rpc_url)?;
            let last_block_num = provider.get_block_number().await?;
            let cast_provider = Cast::new(provider);

            let res = join!(cast_provider.timestamp(last_block_num), cast_provider.timestamp(1));
            let ts_block_latest = res.0.unwrap();
            let ts_block_1 = res.1.unwrap();

            let block_num = if ts_block_latest.lt(&ts_target) {
                // If the most recent block's timestamp is below the target, return it
                last_block_num
            } else if ts_block_1.gt(&ts_target) {
                // If the target timestamp is below block 1's timestamp, return that
                U64::from(1)
            } else {
                // Otherwise, find the block that is closest to the timestamp
                let mut low_block = U64::from(1); // block 0 has a timestamp of 0: https://github.com/ethereum/go-ethereum/issues/17042#issuecomment-559414137
                let mut high_block = last_block_num;
                let mut matching_block: Option<U64> = None;
                while high_block.gt(&low_block) && matching_block.is_none() {
                    // Get timestamp of middle block (this approach approach to avoids overflow)
                    let high_minus_low_over_2 = high_block
                        .checked_sub(low_block)
                        .ok_or_else(|| eyre::eyre!("unexpected underflow"))
                        .unwrap()
                        .checked_div(U64::from(2))
                        .unwrap();
                    let mid_block = high_block.checked_sub(high_minus_low_over_2).unwrap();
                    let ts_mid_block = cast_provider.timestamp(mid_block).await?;

                    // Check if we've found a match or should keep searching
                    if ts_mid_block.eq(&ts_target) {
                        matching_block = Some(mid_block)
                    } else if high_block.checked_sub(low_block).unwrap().eq(&U64::from(1)) {
                        // The target timestamp is in between these blocks. This rounds to the
                        // highest block if timestamp is equidistant between blocks
                        let res = join!(
                            cast_provider.timestamp(high_block),
                            cast_provider.timestamp(low_block)
                        );
                        let ts_high = res.0.unwrap();
                        let ts_low = res.1.unwrap();
                        let high_diff = ts_high.checked_sub(ts_target).unwrap();
                        let low_diff = ts_target.checked_sub(ts_low).unwrap();
                        let is_low = low_diff.lt(&high_diff);
                        matching_block = if is_low { Some(low_block) } else { Some(high_block) }
                    } else if ts_mid_block.lt(&ts_target) {
                        low_block = mid_block;
                    } else {
                        high_block = mid_block;
                    }
                }
                matching_block.unwrap_or(low_block)
            };
            println!("{}", block_num);
        }
        Subcommands::Wallet { command } => match command {
            WalletSubcommands::New { path, password, unsafe_password } => {
                let mut rng = thread_rng();

                match path {
                    Some(path) => {
                        let password = read_secret(password, unsafe_password)?;
                        let (key, uuid) =
                            LocalWallet::new_keystore(&path, &mut rng, password, None)?;
                        let address = SimpleCast::checksum_address(&key.address())?;
                        let filepath = format!(
                            "{}/{}",
                            dunce::canonicalize(path)?
                                .into_os_string()
                                .into_string()
                                .expect("failed to canonicalize file path"),
                            uuid
                        );
                        println!(
                            "Successfully created new keypair at `{}`.\nAddress: {}.",
                            filepath, address
                        );
                    }
                    None => {
                        let wallet = LocalWallet::new(&mut rng);
                        println!(
                            "Successfully created new keypair.\nAddress: {}.\nPrivate Key: {}.",
                            SimpleCast::checksum_address(&wallet.address())?,
                            hex::encode(wallet.signer().to_bytes()),
                        );
                    }
                }
            }
            WalletSubcommands::Vanity { starts_with, ends_with } => {
                let mut regexs = vec![];
                if let Some(prefix) = starts_with {
                    let pad_width = prefix.len() + prefix.len() % 2;
                    hex::decode(format!("{:0>width$}", prefix, width = pad_width))
                        .expect("invalid prefix hex provided");
                    regexs.push(format!(r"^{}", prefix));
                }
                if let Some(suffix) = ends_with {
                    let pad_width = suffix.len() + suffix.len() % 2;
                    hex::decode(format!("{:0>width$}", suffix, width = pad_width))
                        .expect("invalid suffix hex provided");
                    regexs.push(format!(r"{}$", suffix));
                }

                assert!(
                    regexs.iter().map(|p| p.len() - 1).sum::<usize>() <= 40,
                    "vanity patterns length exceeded. cannot be more than 40 characters",
                );

                let regex = RegexSet::new(regexs)?;

                println!("Starting to generate vanity address...");
                let timer = Instant::now();
                let wallet = std::iter::repeat_with(move || LocalWallet::new(&mut thread_rng()))
                    .par_bridge()
                    .find_any(|wallet| {
                        let addr = hex::encode(wallet.address().to_fixed_bytes());
                        regex.matches(&addr).into_iter().count() == regex.patterns().len()
                    })
                    .expect("failed to generate vanity wallet");

                println!(
                    "Successfully created new keypair in {} seconds.\nAddress: {}.\nPrivate Key: {}.",
                    timer.elapsed().as_secs(),
                    SimpleCast::checksum_address(&wallet.address())?,
                    hex::encode(wallet.signer().to_bytes()),
                );
            }
            WalletSubcommands::Address { wallet } => {
                // TODO: Figure out better way to get wallet only.
                let wallet = EthereumOpts {
                    wallet,
                    from: None,
                    rpc_url: Some("http://localhost:8545".to_string()),
                    flashbots: false,
                    chain: Chain::Mainnet,
                    etherscan_api_key: None,
                }
                .signer(0.into())
                .await?
                .unwrap();

                let addr = match wallet {
                    WalletType::Ledger(signer) => signer.address(),
                    WalletType::Local(signer) => signer.address(),
                    WalletType::Trezor(signer) => signer.address(),
                };
                println!("Address: {}", SimpleCast::checksum_address(&addr)?);
            }
            WalletSubcommands::Sign { message, wallet } => {
                // TODO: Figure out better way to get wallet only.
                let wallet = EthereumOpts {
                    wallet,
                    from: None,
                    rpc_url: Some("http://localhost:8545".to_string()),
                    flashbots: false,
                    chain: Chain::Mainnet,
                    etherscan_api_key: None,
                }
                .signer(0.into())
                .await?
                .unwrap();

                let sig = match wallet {
                    WalletType::Ledger(wallet) => wallet.signer().sign_message(&message).await?,
                    WalletType::Local(wallet) => wallet.signer().sign_message(&message).await?,
                    WalletType::Trezor(wallet) => wallet.signer().sign_message(&message).await?,
                };
                println!("Signature: 0x{}", sig);
            }
            WalletSubcommands::Verify { message, signature, address } => {
                let pubkey = Address::from_str(&address).expect("invalid pubkey provided");
                let signature = Signature::from_str(&signature)?;
                match signature.verify(message, pubkey) {
                    Ok(_) => {
                        println!("Validation success. Address {} signed this message.", address)
                    }
                    Err(_) => println!(
                        "Validation failed. Address {} did not sign this message.",
                        address
                    ),
                }
            }
        },
        Subcommands::Completions { shell } => {
            generate(shell, &mut Opts::into_app(), "cast", &mut std::io::stdout())
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

#[allow(clippy::too_many_arguments)]
async fn cast_send<M: Middleware, F: Into<NameOrAddress>, T: Into<NameOrAddress>>(
    provider: M,
    from: F,
    to: T,
    args: (String, Vec<String>),
    gas: Option<U256>,
    gas_price: Option<U256>,
    value: Option<U256>,
    nonce: Option<U256>,
    chain: Chain,
    etherscan_api_key: Option<String>,
    cast_async: bool,
    legacy: bool,
    confs: usize,
    to_json: bool,
) -> eyre::Result<()>
where
    M::Error: 'static,
{
    let cast = Cast::new(provider);

    let sig = args.0;
    let params = args.1;
    let params = if !sig.is_empty() { Some((&sig[..], params)) } else { None };
    let pending_tx = cast
        .send(from, to, params, gas, gas_price, value, nonce, chain, etherscan_api_key, legacy)
        .await?;
    let tx_hash = *pending_tx;

    if cast_async {
        println!("{:#x}", tx_hash);
    } else {
        let receipt = cast.receipt(format!("{:#x}", tx_hash), None, confs, false, to_json).await?;
        println!("{}", receipt);
    }

    Ok(())
}
