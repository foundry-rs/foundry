use crate::SimpleCast;
use alloy_primitives::{Address, hex};
use clap::Parser;
use eyre::Result;
use foundry_common::{sh_println, stdin};
use std::{env, fs};

/// CLI arguments for `cast --to-base`.
#[derive(Debug, Parser)]
pub struct ToBaseArgs {
    /// The value to convert.
    #[arg(allow_hyphen_values = true)]
    pub value: Option<String>,

    /// The input base.
    #[arg(long, short = 'i')]
    pub base_in: Option<String>,
}
#[derive(Debug, Parser)]
pub enum ConvertSubCommand {
    /// Convert wei into an ETH amount.
    ///
    /// Consider using --to-unit.
    #[command(visible_aliases = &["--from-wei", "fw"])]
    FromWei {
        /// The value to convert.
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,

        /// The unit to convert from (ether, gwei, wei).
        #[arg(default_value = "eth")]
        unit: String,
    },
    /// Convert an ETH amount to wei.
    ///
    /// Consider using --to-unit.
    #[command(visible_aliases = &["--to-wei", "tw", "2w"])]
    ToWei {
        /// The value to convert.
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,

        /// The unit to convert from (ether, gwei, wei).
        #[arg(default_value = "eth")]
        unit: String,
    },
    /// Convert an ETH amount into another unit (ether, gwei or wei).
    ///
    /// Examples:
    /// - 1ether wei
    /// - "1 ether" wei
    /// - 1ether
    /// - 1 gwei
    /// - 1gwei ether
    #[command(visible_aliases = &["--to-unit", "tun", "2un"])]
    ToUnit {
        /// The value to convert.
        value: Option<String>,

        /// The unit to convert to (ether, gwei, wei).
        #[arg(default_value = "wei")]
        unit: String,
    },
    /// Convert an integer into a fixed point number.
    #[command(visible_aliases = &["--to-fix", "tf", "2f"])]
    ToFixedPoint {
        /// The number of decimals to use.
        decimals: Option<String>,

        /// The value to convert.
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,
    },
    /// Convert a fixed point number into an integer.
    #[command(visible_aliases = &["--from-fix", "ff"])]
    FromFixedPoint {
        /// The number of decimals to use.
        decimals: Option<String>,

        /// The value to convert.
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,
    },
    /// Format a number from smallest unit to decimal with arbitrary decimals.
    ///
    /// Examples:
    /// - 1000000 6       (for USDC, result: 1.0)
    /// - 2500000000000 12 (for 12 decimals, result: 2.5)
    /// - 1230 3          (for 3 decimals, result: 1.23)
    #[command(visible_aliases = &["--format-units", "fun"])]
    FormatUnits {
        /// The value to format.
        value: Option<String>,

        /// The unit to format to.
        #[arg(default_value = "18")]
        unit: u8,
    },
    /// Convert a number from decimal to smallest unit with arbitrary decimals.
    ///
    /// Examples:
    /// - 1.0 6    (for USDC, result: 1000000)
    /// - 2.5 12   (for 12 decimals token, result: 2500000000000)
    /// - 1.23 3   (for 3 decimals token, result: 1230)
    #[command(visible_aliases = &["--parse-units", "pun"])]
    ParseUnits {
        /// The value to convert.
        value: Option<String>,

        /// The unit to convert to.
        #[arg(default_value = "18")]
        unit: u8,
    },
    /// Converts a number of one base to another
    #[command(visible_aliases = &["--to-hex", "th", "2h"])]
    ToHex(ToBaseArgs),

    /// Converts a number of one base to decimal
    #[command(visible_aliases = &["--to-dec", "td", "2d"])]
    ToDec(ToBaseArgs),
    /// Converts a number of one base to another
    #[command(
        visible_aliases = &["--to-base",
        "--to-radix",
        "to-radix",
        "tr",
        "2r"]
    )]
    ToBase {
        #[command(flatten)]
        base: ToBaseArgs,

        /// The output base.
        #[arg(value_name = "BASE")]
        base_out: Option<String>,
    },
    /// Convert a number to a hex-encoded int256.
    #[command(name = "to-int256", visible_aliases = &["--to-int256", "ti", "2i"])]
    ToInt256 {
        /// The value to convert.
        value: Option<String>,
    },
    /// Convert a number to a hex-encoded uint256.
    #[command(name = "to-uint256", visible_aliases = &["--to-uint256", "tu", "2u"])]
    ToUint256 {
        /// The value to convert.
        value: Option<String>,
    },

    // New commands copied from opts.rs
    /// Convert UTF-8 text to hex.
    #[command(
        visible_aliases = &[
        "--from-ascii",
        "--from-utf8",
        "from-ascii",
        "fu",
        "fa"]
    )]
    FromUtf8 {
        /// The text to convert.
        text: Option<String>,
    },

    /// Convert hex data to a utf-8 string.
    #[command(visible_aliases = &["--to-utf8", "tu8", "2u8"])]
    ToUtf8 {
        /// The hex data to convert.
        hexdata: Option<String>,
    },

    /// Convert hex data to an ASCII string.
    #[command(visible_aliases = &["--to-ascii", "tas", "2as"])]
    ToAscii {
        /// The hex data to convert.
        hexdata: Option<String>,
    },

    /// Convert binary data into hex data.
    #[command(visible_aliases = &["--from-bin", "from-binx", "fb"])]
    FromBin,

    /// Right-pads hex data to 32 bytes.
    #[command(visible_aliases = &["--to-bytes32", "tb", "2b"])]
    ToBytes32 {
        /// The hex data to convert.
        bytes: Option<String>,
    },

    /// Normalize the input to lowercase, 0x-prefixed hex.
    ///
    /// The input can be:
    /// - mixed case hex with or without 0x prefix
    /// - 0x prefixed hex, concatenated with a ':'
    /// - an absolute path to file
    /// - @tag, where the tag is defined in an environment variable
    #[command(visible_aliases = &["--to-hexdata", "thd", "2hd"])]
    ToHexdata {
        /// The input to normalize.
        input: Option<String>,
    },

    /// Formats a string into bytes32 encoding.
    #[command(name = "format-bytes32-string", visible_aliases = &["--format-bytes32-string"])]
    FormatBytes32String {
        /// The string to format.
        string: Option<String>,
    },

    /// Parses a string from bytes32 encoding.
    #[command(name = "parse-bytes32-string", visible_aliases = &["--parse-bytes32-string"])]
    ParseBytes32String {
        /// The string to parse.
        bytes: Option<String>,
    },

    /// Parses a checksummed address from bytes32 encoding.
    #[command(name = "parse-bytes32-address", visible_aliases = &["--parse-bytes32-address"])]
    ParseBytes32Address {
        #[arg(value_name = "BYTES")]
        bytes: Option<String>,
    },

    /// Convert an address to a checksummed format (EIP-55).
    #[command(
        visible_aliases = &["--to-checksum-address",
        "--to-checksum",
        "to-checksum",
        "ta",
        "2a"]
    )]
    ToCheckSumAddress {
        /// The address to convert.
        address: Option<Address>,
        /// EIP-155 chain ID to encode the address using EIP-1191.
        chain_id: Option<u64>,
    },

    /// Concatenate hex strings.
    #[command(visible_aliases = &["--concat-hex", "ch"])]
    ConcatHex {
        /// The data to concatenate.
        data: Vec<String>,
    },

    /// Pads hex data to a specified length.
    #[command(visible_aliases = &["pd"])]
    Pad {
        /// The hex data to pad.
        data: Option<String>,

        /// Right-pad the data (instead of left-pad).
        #[arg(long)]
        right: bool,

        /// Left-pad the data (default).
        #[arg(long, conflicts_with = "right")]
        left: bool,

        /// Target length in bytes (default: 32).
        #[arg(long, default_value = "32")]
        len: usize,
    },

    /// RLP encode the given JSON data.
    ///
    /// Example: cast to-rlp "[\"0x61\"]" -> `0xc161`
    /// - `cast to-rlp "[\"0x61\"]"` -> `0xc161`
    /// - `cast to-rlp "[\"0xf1\", \"f2\"]"` -> `0xc481f181f2`
    #[command(visible_aliases = &["--to-rlp"])]
    ToRlp {
        /// The value to convert.
        ///
        /// This is a hex-encoded string, or an array of hex-encoded strings.
        /// Can be arbitrarily recursive.
        value: Option<String>,
    },

    /// Decodes RLP hex-encoded data.
    #[command(visible_aliases = &["--from-rlp"])]
    FromRlp {
        /// The RLP hex-encoded data.
        value: Option<String>,

        /// Decode the RLP data as int
        #[arg(long, alias = "int")]
        as_int: bool,
    },
}

impl ConvertSubCommand {
    pub async fn run(self) -> Result<()> {
        match self {
            // Wei/Ether unit conversions
            Self::FromWei { value, unit } => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::from_wei(&value, &unit)?)?;
            }
            Self::ToWei { value, unit } => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::to_wei(&value, &unit)?)?;
            }
            Self::ToUnit { value, unit } => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::to_unit(&value, &unit)?)?;
            }

            // Fixed point conversions
            Self::ToFixedPoint { value, decimals } => {
                let (value, decimals) = stdin::unwrap2(value, decimals)?;
                sh_println!("{}", SimpleCast::to_fixed_point(&value, &decimals)?)?;
            }
            Self::FromFixedPoint { value, decimals } => {
                let (value, decimals) = stdin::unwrap2(value, decimals)?;
                sh_println!("{}", SimpleCast::from_fixed_point(&value, &decimals)?)?;
            }

            // Unit parsing and formatting
            Self::FormatUnits { value, unit } => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::format_units(&value, unit)?)?;
            }
            Self::ParseUnits { value, unit } => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::parse_units(&value, unit)?)?;
            }

            // Base conversions
            Self::ToHex(ToBaseArgs { value, base_in }) => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::to_base(&value, base_in.as_deref(), "hex")?)?;
            }
            Self::ToDec(ToBaseArgs { value, base_in }) => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::to_base(&value, base_in.as_deref(), "dec")?)?;
            }
            Self::ToBase { base: ToBaseArgs { value, base_in }, base_out } => {
                let (value, base_out) = stdin::unwrap2(value, base_out)?;
                sh_println!("{}", SimpleCast::to_base(&value, base_in.as_deref(), &base_out)?)?;
            }

            // Integer conversions
            Self::ToInt256 { value } => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::to_int256(&value)?)?;
            }
            Self::ToUint256 { value } => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::to_uint256(&value)?)?;
            }

            // String/Hex conversions
            Self::FromUtf8 { text } => {
                let value = stdin::unwrap(text, false)?;
                sh_println!("{}", SimpleCast::from_utf8(&value))?;
            }
            Self::ToUtf8 { hexdata } => {
                let value = stdin::unwrap(hexdata, false)?;
                sh_println!("{}", SimpleCast::to_utf8(&value)?)?;
            }
            Self::ToAscii { hexdata } => {
                let value = stdin::unwrap(hexdata, false)?;
                sh_println!("{}", SimpleCast::to_ascii(value.trim())?)?;
            }

            // Binary conversion
            Self::FromBin => {
                let hex = stdin::read_bytes(false)?;
                sh_println!("{}", hex::encode_prefixed(hex))?;
            }

            // Bytes32 conversions
            Self::ToBytes32 { bytes } => {
                let value = stdin::unwrap_line(bytes)?;
                sh_println!("{}", SimpleCast::to_bytes32(&value)?)?;
            }

            // Hex data normalization
            Self::ToHexdata { input } => {
                let value = stdin::unwrap_line(input)?;
                let output = match value {
                    s if s.starts_with('@') => hex::encode(env::var(&s[1..])?),
                    s if s.starts_with('/') => hex::encode(fs::read(s)?),
                    s => s.split(':').map(|s| s.trim_start_matches("0x").to_lowercase()).collect(),
                };
                sh_println!("0x{output}")?;
            }

            // Bytes32 string operations
            Self::FormatBytes32String { string } => {
                let value = stdin::unwrap_line(string)?;
                sh_println!("{}", SimpleCast::format_bytes32_string(&value)?)?;
            }
            Self::ParseBytes32String { bytes } => {
                let value = stdin::unwrap_line(bytes)?;
                sh_println!("{}", SimpleCast::parse_bytes32_string(&value)?)?;
            }
            Self::ParseBytes32Address { bytes } => {
                let value = stdin::unwrap_line(bytes)?;
                sh_println!("{}", SimpleCast::parse_bytes32_address(&value)?)?;
            }

            // Address checksum
            Self::ToCheckSumAddress { address, chain_id } => {
                let value = stdin::unwrap_line(address)?;
                sh_println!("{}", value.to_checksum(chain_id))?;
            }

            // Hex operations
            Self::ConcatHex { data } => {
                if data.is_empty() {
                    let s = stdin::read(true)?;
                    sh_println!("{}", SimpleCast::concat_hex(s.split_whitespace()))?;
                } else {
                    sh_println!("{}", SimpleCast::concat_hex(data))?;
                }
            }
            Self::Pad { data, right, left: _, len } => {
                let value = stdin::unwrap_line(data)?;
                sh_println!("{}", SimpleCast::pad(&value, right, len)?)?;
            }

            // RLP operations
            Self::ToRlp { value } => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::to_rlp(&value)?)?;
            }
            Self::FromRlp { value, as_int } => {
                let value = stdin::unwrap_line(value)?;
                sh_println!("{}", SimpleCast::from_rlp(value, as_int)?)?;
            }
        }
        Ok(())
    }
}
