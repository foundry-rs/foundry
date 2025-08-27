use alloy_dyn_abi::JsonAbiExt;
use alloy_primitives::U256;
use std::str::FromStr;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(s: &str);
}

#[wasm_bindgen]
pub fn calldata_encode(signature: &str, args: JsValue) -> Result<String, JsValue> {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::{hex, utils::keccak256};

    // Parse signature
    let (name, params) =
        signature.split_once('(').ok_or_else(|| JsValue::from_str("invalid function signature"))?;
    let params = params.strip_suffix(')').unwrap_or(params);
    let type_strs: Vec<&str> = if params.trim().is_empty() {
        vec![]
    } else {
        params.split(',').map(|s| s.trim()).collect()
    };

    // Parse types and args
    let types: Vec<DynSolType> = type_strs
        .iter()
        .map(|t| DynSolType::parse(t).map_err(|e| JsValue::from_str(&format!("{e}"))))
        .collect::<Result<_, _>>()?;
    let args_vec: Vec<String> = serde_wasm_bindgen::from_value(args)
        .map_err(|e| JsValue::from_str(&format!("invalid args array: {e}")))?;
    if args_vec.len() != types.len() {
        return Err(JsValue::from_str("argument count mismatch"));
    }
    let values: Vec<DynSolValue> = types
        .iter()
        .zip(args_vec.iter())
        .map(|(ty, s)| {
            DynSolType::coerce_str(ty, s).map_err(|e| JsValue::from_str(&format!("{e}")))
        })
        .collect::<Result<_, _>>()?;

    // Encode
    let selector_sig = format!("{name}({})", type_strs.join(","));
    let mut out = Vec::with_capacity(4 + 32 * values.len());
    out.extend_from_slice(&keccak256(selector_sig.as_bytes())[..4]);
    let data = DynSolValue::Tuple(values).abi_encode();
    out.extend_from_slice(&data);
    Ok(format!("0x{}", hex::encode(out)))
}

#[wasm_bindgen]
pub fn calldata_decode(signature: &str, calldata: &str) -> Result<String, JsValue> {
    use alloy_json_abi::Function;
    use alloy_primitives::hex;

    // Parse the function signature
    let func = Function::parse(signature)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse signature: {e}")))?;

    let bytes = hex::decode(calldata)
        .map_err(|e| JsValue::from_str(&format!("Failed to decode calldata: {e}")))?;

    // Skip function selector (first 4 bytes) if present
    let data = if bytes.len() >= 4 { &bytes[4..] } else { &bytes[..] };

    // Decode the input data
    let decoded = func
        .abi_decode_input(data)
        .map_err(|e| JsValue::from_str(&format!("Failed to decode: {e}")))?;

    // Format output
    let output: Vec<String> = decoded.iter().map(|v| format!("{:?}", v)).collect();
    Ok(serde_json::to_string(&output).unwrap())
}

#[wasm_bindgen]
pub fn abi_decode(signature: &str, data: &str) -> Result<String, JsValue> {
    use alloy_dyn_abi::DynSolType;
    use alloy_primitives::hex;

    // Parse signature to get parameter types (handle both input and output forms)
    let params = if let Some((_, rest)) = signature.split_once('(') {
        // Check if it has output types after )
        if let Some((input_params, output_part)) = rest.split_once(')') {
            // Check if there's an output specification
            if output_part.starts_with('(') {
                // Has output types, use those for decoding
                output_part.strip_prefix('(').and_then(|s| s.strip_suffix(')')).unwrap_or("")
            } else {
                // No output types, use input types
                input_params
            }
        } else {
            rest.strip_suffix(')').unwrap_or(rest)
        }
    } else {
        signature
    };

    let type_strs: Vec<&str> = if params.trim().is_empty() {
        vec![]
    } else {
        params.split(',').map(|s| s.trim()).collect()
    };

    // Parse types
    let types: Vec<DynSolType> = type_strs
        .iter()
        .map(|t| {
            DynSolType::parse(t)
                .map_err(|e| JsValue::from_str(&format!("Failed to parse type: {e}")))
        })
        .collect::<Result<_, _>>()?;

    // Decode the data
    let bytes =
        hex::decode(data).map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?;

    let decoded = if types.len() == 1 {
        vec![
            types[0]
                .abi_decode(&bytes)
                .map_err(|e| JsValue::from_str(&format!("Failed to decode: {e}")))?,
        ]
    } else {
        types
            .iter()
            .zip(bytes.chunks(32))
            .map(|(ty, chunk)| {
                ty.abi_decode(chunk)
                    .map_err(|e| JsValue::from_str(&format!("Failed to decode: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    // Format output
    let output: Vec<String> = decoded.iter().map(|v| format!("{:?}", v)).collect();
    Ok(serde_json::to_string(&output).unwrap())
}

#[wasm_bindgen]
pub fn abi_encode(signature: &str, args: JsValue) -> Result<String, JsValue> {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::hex;

    // Parse signature to get parameter types
    let params = if let Some((_name, params)) = signature.split_once('(') {
        params.strip_suffix(')').unwrap_or(params)
    } else {
        signature
    };

    let type_strs: Vec<&str> = if params.trim().is_empty() {
        vec![]
    } else {
        params.split(',').map(|s| s.trim()).collect()
    };

    // Parse types and args
    let types: Vec<DynSolType> = type_strs
        .iter()
        .map(|t| {
            DynSolType::parse(t)
                .map_err(|e| JsValue::from_str(&format!("Failed to parse type: {e}")))
        })
        .collect::<Result<_, _>>()?;

    let args_vec: Vec<String> = serde_wasm_bindgen::from_value(args)
        .map_err(|e| JsValue::from_str(&format!("Invalid args array: {e}")))?;

    if args_vec.len() != types.len() {
        return Err(JsValue::from_str("Argument count mismatch"));
    }

    let values: Vec<DynSolValue> = types
        .iter()
        .zip(args_vec.iter())
        .map(|(ty, s)| {
            DynSolType::coerce_str(ty, s)
                .map_err(|e| JsValue::from_str(&format!("Failed to coerce value: {e}")))
        })
        .collect::<Result<_, _>>()?;

    // Encode
    let encoded = DynSolValue::Tuple(values).abi_encode();
    Ok(format!("0x{}", hex::encode(encoded)))
}

#[wasm_bindgen]
pub fn keccak256(data: &str) -> Result<String, JsValue> {
    use alloy_primitives::{hex, utils::keccak256};

    // Decode hex if it starts with 0x, otherwise treat as UTF-8
    let bytes = if data.starts_with("0x") || data.starts_with("0X") {
        hex::decode(data).map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?
    } else {
        data.as_bytes().to_vec()
    };

    let hash = keccak256(&bytes);
    Ok(format!("0x{}", hex::encode(hash)))
}

#[wasm_bindgen]
pub fn to_hex(value: &str) -> Result<String, JsValue> {
    use alloy_primitives::hex;

    // Try to parse as number first
    if let Ok(num) = U256::from_str(value) {
        return Ok(format!("{num:#x}"));
    }

    // Otherwise encode as hex string
    Ok(format!("0x{}", hex::encode(value)))
}

#[wasm_bindgen]
pub fn from_hex(hex_str: &str) -> Result<String, JsValue> {
    use alloy_primitives::hex;

    let bytes = hex::decode(hex_str)
        .map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?;

    // Try to convert to UTF-8 string
    String::from_utf8(bytes).map_err(|e| JsValue::from_str(&format!("Invalid UTF-8: {e}")))
}

#[wasm_bindgen]
pub fn to_uint256(value: &str) -> Result<String, JsValue> {
    // Parse with base detection
    let num = if value.starts_with("0x") || value.starts_with("0X") {
        U256::from_str_radix(value, 16)
    } else if value.starts_with("0b") || value.starts_with("0B") {
        U256::from_str_radix(&value[2..], 2)
    } else if value.starts_with("0o") || value.starts_with("0O") {
        U256::from_str_radix(&value[2..], 8)
    } else {
        U256::from_str(value)
    };

    let n = num.map_err(|e| JsValue::from_str(&format!("Failed to parse number: {e}")))?;
    Ok(format!("{n:#066x}"))
}

#[wasm_bindgen]
pub fn to_int256(value: &str) -> Result<String, JsValue> {
    // Parse as signed integer
    let is_negative = value.starts_with('-');
    let abs_value = if is_negative { &value[1..] } else { value };

    // Parse absolute value
    let num = if abs_value.starts_with("0x") || abs_value.starts_with("0X") {
        U256::from_str_radix(abs_value, 16)
    } else if abs_value.starts_with("0b") || abs_value.starts_with("0B") {
        U256::from_str_radix(&abs_value[2..], 2)
    } else if abs_value.starts_with("0o") || abs_value.starts_with("0O") {
        U256::from_str_radix(&abs_value[2..], 8)
    } else {
        U256::from_str(abs_value)
    };

    let n = num.map_err(|e| JsValue::from_str(&format!("Failed to parse number: {e}")))?;

    if is_negative {
        // Two's complement for negative numbers
        let neg = (!n).wrapping_add(U256::from(1));
        Ok(format!("{neg:#066x}"))
    } else {
        Ok(format!("{n:#066x}"))
    }
}

#[wasm_bindgen]
pub fn format_bytes32_string(s: &str) -> Result<String, JsValue> {
    let bytes = s.as_bytes();
    if bytes.len() > 32 {
        return Err(JsValue::from_str("String exceeds 32 bytes"));
    }

    let mut result = [0u8; 32];
    result[..bytes.len()].copy_from_slice(bytes);

    Ok(format!("0x{}", alloy_primitives::hex::encode(result)))
}

#[wasm_bindgen]
pub fn parse_bytes32_string(hex_str: &str) -> Result<String, JsValue> {
    use alloy_primitives::hex;

    let bytes = hex::decode(hex_str)
        .map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?;

    if bytes.len() != 32 {
        return Err(JsValue::from_str("Expected 32 bytes"));
    }

    // Find null terminator
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(32);

    String::from_utf8(bytes[..end].to_vec())
        .map_err(|e| JsValue::from_str(&format!("Invalid UTF-8: {e}")))
}

#[wasm_bindgen]
pub fn concat_hex(values: JsValue) -> Result<String, JsValue> {
    use alloy_primitives::hex;

    let hex_values: Vec<String> = serde_wasm_bindgen::from_value(values)
        .map_err(|e| JsValue::from_str(&format!("Invalid input array: {e}")))?;

    let mut result = Vec::new();
    for hex_str in hex_values {
        let bytes = hex::decode(&hex_str)
            .map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?;
        result.extend_from_slice(&bytes);
    }

    Ok(format!("0x{}", hex::encode(result)))
}

#[wasm_bindgen]
pub fn left_shift(value: &str, bits: &str) -> Result<String, JsValue> {
    let val = U256::from_str(value)
        .or_else(|_| U256::from_str_radix(value, 16))
        .map_err(|e| JsValue::from_str(&format!("Failed to parse value: {e}")))?;

    let shift_amount = usize::from_str(bits)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse bits: {e}")))?;

    if shift_amount >= 256 {
        return Ok(format!("{:#066x}", U256::ZERO));
    }

    let result = val << shift_amount;
    Ok(format!("{result:#066x}"))
}

#[wasm_bindgen]
pub fn right_shift(value: &str, bits: &str) -> Result<String, JsValue> {
    let val = U256::from_str(value)
        .or_else(|_| U256::from_str_radix(value, 16))
        .map_err(|e| JsValue::from_str(&format!("Failed to parse value: {e}")))?;

    let shift_amount = usize::from_str(bits)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse bits: {e}")))?;

    if shift_amount >= 256 {
        return Ok(format!("{:#066x}", U256::ZERO));
    }

    let result = val >> shift_amount;
    Ok(format!("{result:#066x}"))
}

#[wasm_bindgen]
pub fn to_ascii(hex_str: &str) -> Result<String, JsValue> {
    use alloy_primitives::hex;

    let bytes = hex::decode(hex_str)
        .map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?;

    // Check all bytes are ASCII
    if !bytes.iter().all(|b| b.is_ascii()) {
        return Err(JsValue::from_str("Input contains non-ASCII bytes"));
    }

    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[wasm_bindgen]
pub fn from_utf8(s: &str) -> String {
    format!("0x{}", alloy_primitives::hex::encode(s.as_bytes()))
}

#[wasm_bindgen]
pub fn to_utf8(hex_str: &str) -> Result<String, JsValue> {
    use alloy_primitives::hex;

    let bytes = hex::decode(hex_str)
        .map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?;

    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[wasm_bindgen]
pub fn selector(signature: &str) -> Result<String, JsValue> {
    use alloy_primitives::utils::keccak256;

    let hash = keccak256(signature.as_bytes());
    Ok(format!("0x{}", alloy_primitives::hex::encode(&hash[..4])))
}

#[wasm_bindgen]
pub fn pad_left(s: &str, length: usize) -> Result<String, JsValue> {
    use alloy_primitives::hex;

    let bytes =
        hex::decode(s).map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?;

    if bytes.len() > length {
        return Err(JsValue::from_str(&format!("Input exceeds target length of {} bytes", length)));
    }

    let mut padded = vec![0u8; length];
    let start = length - bytes.len();
    padded[start..].copy_from_slice(&bytes);

    Ok(format!("0x{}", hex::encode(padded)))
}

#[wasm_bindgen]
pub fn pad_right(s: &str, length: usize) -> Result<String, JsValue> {
    use alloy_primitives::hex;

    let bytes =
        hex::decode(s).map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?;

    if bytes.len() > length {
        return Err(JsValue::from_str(&format!("Input exceeds target length of {} bytes", length)));
    }

    let mut padded = vec![0u8; length];
    padded[..bytes.len()].copy_from_slice(&bytes);

    Ok(format!("0x{}", hex::encode(padded)))
}

#[wasm_bindgen]
pub fn to_wei(value: &str, unit: &str) -> Result<String, JsValue> {
    use alloy_primitives::utils::{ParseUnits, Unit};

    let unit =
        unit.parse::<Unit>().map_err(|e| JsValue::from_str(&format!("Invalid unit: {e}")))?;

    let result = ParseUnits::parse_units(value, unit)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse: {e}")))?;

    Ok(result.to_string())
}

#[wasm_bindgen]
pub fn from_wei(value: &str, unit: &str) -> Result<String, JsValue> {
    use alloy_primitives::utils::{ParseUnits, Unit};

    let parsed_value = U256::from_str(value)
        .or_else(|_| U256::from_str_radix(value, 16))
        .map_err(|e| JsValue::from_str(&format!("Failed to parse value: {e}")))?;

    let unit =
        unit.parse::<Unit>().map_err(|e| JsValue::from_str(&format!("Invalid unit: {e}")))?;

    Ok(ParseUnits::U256(parsed_value).format_units(unit))
}

#[wasm_bindgen]
pub fn parse_units(value: &str, decimals: u8) -> Result<String, JsValue> {
    use alloy_primitives::utils::{ParseUnits, Unit};

    let unit = Unit::new(decimals).ok_or_else(|| JsValue::from_str("Invalid decimals"))?;

    let result = ParseUnits::parse_units(value, unit)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse: {e}")))?;

    Ok(result.to_string())
}

#[wasm_bindgen]
pub fn format_units(value: &str, decimals: u8) -> Result<String, JsValue> {
    use alloy_primitives::utils::{ParseUnits, Unit};

    let parsed_value = U256::from_str(value)
        .or_else(|_| U256::from_str_radix(value, 16))
        .map_err(|e| JsValue::from_str(&format!("Failed to parse value: {e}")))?;

    let unit = Unit::new(decimals).ok_or_else(|| JsValue::from_str("Invalid decimals"))?;

    Ok(ParseUnits::U256(parsed_value).format_units(unit))
}

#[wasm_bindgen]
pub fn max_int(bits: &str) -> Result<String, JsValue> {
    let bits_num =
        usize::from_str(bits).map_err(|e| JsValue::from_str(&format!("Invalid bits: {e}")))?;

    if bits_num == 0 || bits_num > 256 || bits_num % 8 != 0 {
        return Err(JsValue::from_str("Invalid bit size"));
    }

    // For signed integers, max is 2^(bits-1) - 1
    let max = if bits_num == 256 {
        // Special case for 256 bits to avoid overflow
        U256::from_str("0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
            .unwrap()
    } else {
        (U256::from(1) << (bits_num - 1)) - U256::from(1)
    };

    Ok(max.to_string())
}

#[wasm_bindgen]
pub fn min_int(bits: &str) -> Result<String, JsValue> {
    let bits_num =
        usize::from_str(bits).map_err(|e| JsValue::from_str(&format!("Invalid bits: {e}")))?;

    if bits_num == 0 || bits_num > 256 || bits_num % 8 != 0 {
        return Err(JsValue::from_str("Invalid bit size"));
    }

    // For signed integers, min is -2^(bits-1)
    let min = if bits_num == 256 {
        // Special case for 256 bits
        U256::from_str("0x8000000000000000000000000000000000000000000000000000000000000000")
            .unwrap()
    } else {
        U256::from(1) << (bits_num - 1)
    };

    // Return as negative (two's complement)
    Ok(format!("-{}", min))
}

#[wasm_bindgen]
pub fn to_base(value: &str, base_out: &str) -> Result<String, JsValue> {
    // Auto-detect input base
    let num = if value.starts_with("0x") || value.starts_with("0X") {
        let hex_val = if value.len() > 2 { &value[2..] } else { "0" };
        U256::from_str_radix(hex_val, 16)
    } else if value.starts_with("0b") || value.starts_with("0B") {
        let bin_val = if value.len() > 2 { &value[2..] } else { "0" };
        U256::from_str_radix(bin_val, 2)
    } else if value.starts_with("0o") || value.starts_with("0O") {
        let oct_val = if value.len() > 2 { &value[2..] } else { "0" };
        U256::from_str_radix(oct_val, 8)
    } else {
        U256::from_str(value)
    }
    .map_err(|e| JsValue::from_str(&format!("Failed to parse value: {e}")))?;

    // Convert to output base
    match base_out.to_lowercase().as_str() {
        "bin" | "binary" | "2" => Ok(format!("0b{:b}", num)),
        "oct" | "octal" | "8" => Ok(format!("0o{:o}", num)),
        "dec" | "decimal" | "10" => Ok(num.to_string()),
        "hex" | "hexadecimal" | "16" => Ok(format!("{:#x}", num)),
        _ => Err(JsValue::from_str("Invalid output base")),
    }
}

#[wasm_bindgen]
pub fn abi_encode_packed(signature: &str, args: JsValue) -> Result<String, JsValue> {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::hex;

    // Parse signature to get parameter types
    let sig =
        if !signature.starts_with('(') { format!("({signature})") } else { signature.to_string() };

    let params = sig.strip_prefix('(').and_then(|s| s.strip_suffix(')')).unwrap_or(&sig);

    let type_strs: Vec<&str> = if params.trim().is_empty() {
        vec![]
    } else {
        params.split(',').map(|s| s.trim()).collect()
    };

    // Parse types and args
    let types: Vec<DynSolType> = type_strs
        .iter()
        .map(|t| {
            DynSolType::parse(t)
                .map_err(|e| JsValue::from_str(&format!("Failed to parse type: {e}")))
        })
        .collect::<Result<_, _>>()?;

    let args_vec: Vec<String> = serde_wasm_bindgen::from_value(args)
        .map_err(|e| JsValue::from_str(&format!("Invalid args array: {e}")))?;

    if args_vec.len() != types.len() {
        return Err(JsValue::from_str("Argument count mismatch"));
    }

    // Encode packed (concatenate without padding)
    let mut packed = Vec::new();
    for (ty, arg) in types.iter().zip(args_vec.iter()) {
        let value = DynSolType::coerce_str(ty, arg)
            .map_err(|e| JsValue::from_str(&format!("Failed to coerce value: {e}")))?;

        // For packed encoding, we need to handle each type specially
        let encoded = match &value {
            DynSolValue::Address(addr) => addr.to_vec(),
            DynSolValue::Bool(b) => vec![if *b { 1 } else { 0 }],
            DynSolValue::Bytes(b) => b.to_vec(),
            DynSolValue::FixedBytes(b, _) => b.to_vec(),
            DynSolValue::Int(i, bits) => {
                let bytes = (*bits / 8) as usize;
                let mut buf = i.to_be_bytes::<32>().to_vec();
                buf.drain(0..(32 - bytes));
                buf
            }
            DynSolValue::Uint(u, bits) => {
                let bytes = (*bits / 8) as usize;
                let mut buf = u.to_be_bytes::<32>().to_vec();
                buf.drain(0..(32 - bytes));
                buf
            }
            DynSolValue::String(s) => s.as_bytes().to_vec(),
            _ => value.abi_encode_packed(),
        };

        packed.extend_from_slice(&encoded);
    }

    Ok(format!("0x{}", hex::encode(packed)))
}

#[wasm_bindgen]
pub fn storage_index(key: &str, slot: &str) -> Result<String, JsValue> {
    use alloy_primitives::{hex, utils::keccak256};

    // Parse slot number
    let slot_num = U256::from_str(slot)
        .or_else(|_| U256::from_str_radix(slot, 16))
        .map_err(|e| JsValue::from_str(&format!("Failed to parse slot: {e}")))?;

    // Parse key (could be address, uint, etc)
    let key_bytes = if key.starts_with("0x") {
        hex::decode(key).map_err(|e| JsValue::from_str(&format!("Failed to decode key: {e}")))?
    } else {
        // Try to parse as number first
        if let Ok(num) = U256::from_str(key) {
            num.to_be_bytes::<32>().to_vec()
        } else {
            // Treat as string/bytes
            key.as_bytes().to_vec()
        }
    };

    // Pad key to 32 bytes
    let mut key_padded = vec![0u8; 32];
    let copy_len = key_bytes.len().min(32);
    key_padded[(32 - copy_len)..].copy_from_slice(&key_bytes[..copy_len]);

    // Concatenate key and slot, then hash
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(&key_padded);
    data.extend_from_slice(&slot_num.to_be_bytes::<32>());

    let hash = keccak256(&data);
    Ok(format!("0x{}", hex::encode(hash)))
}

#[wasm_bindgen]
pub fn parse_bytes32_address(hex_str: &str) -> Result<String, JsValue> {
    use alloy_primitives::{Address, hex};

    let bytes = hex::decode(hex_str)
        .map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?;

    if bytes.len() != 32 {
        return Err(JsValue::from_str("Expected 32 bytes"));
    }

    // Extract last 20 bytes as address
    let addr_bytes = &bytes[12..32];
    let address = Address::from_slice(addr_bytes);

    Ok(format!("{}", address))
}

#[wasm_bindgen]
pub fn to_bytes32(s: &str) -> Result<String, JsValue> {
    use alloy_primitives::hex;

    let bytes = if s.starts_with("0x") {
        hex::decode(s).map_err(|e| JsValue::from_str(&format!("Failed to decode hex: {e}")))?
    } else {
        s.as_bytes().to_vec()
    };

    if bytes.len() > 32 {
        return Err(JsValue::from_str("Data exceeds 32 bytes"));
    }

    let mut result = vec![0u8; 32];
    result[..bytes.len()].copy_from_slice(&bytes);

    Ok(format!("0x{}", hex::encode(result)))
}

#[wasm_bindgen]
pub async fn rpc(url: String, method: String, params: JsValue) -> Result<JsValue, JsValue> {
    use js_sys::Promise;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Headers, Request, RequestInit, Response};

    // Prepare JSON-RPC body
    let params_js = params;
    let body = js_sys::Object::new();
    js_sys::Reflect::set(&body, &JsValue::from_str("jsonrpc"), &JsValue::from_str("2.0")).unwrap();
    js_sys::Reflect::set(&body, &JsValue::from_str("id"), &JsValue::from_f64(1.0)).unwrap();
    js_sys::Reflect::set(&body, &JsValue::from_str("method"), &JsValue::from_str(&method)).unwrap();
    js_sys::Reflect::set(&body, &JsValue::from_str("params"), &params_js).unwrap();
    let body_str = js_sys::JSON::stringify(&body)
        .map_err(|e| JsValue::from_str(&format!("failed to stringify body: {e:?}")))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("failed to stringify body"))?;

    // Build request
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_body(&JsValue::from_str(&body_str));
    let request = Request::new_with_str_and_init(&url, &init)
        .map_err(|e| JsValue::from_str(&format!("failed to build request: {e:?}")))?;
    let headers = Headers::new().unwrap();
    headers.set("Content-Type", "application/json").unwrap();
    request.headers().set("Content-Type", "application/json").unwrap();

    // Fetch
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let resp_value = JsFuture::from(Promise::from(window.fetch_with_request(&request)))
        .await
        .map_err(|e| JsValue::from_str(&format!("fetch failed: {e:?}")))?;
    let resp: Response = resp_value.dyn_into().unwrap();
    if !resp.ok() {
        let status = resp.status();
        return Err(JsValue::from_str(&format!("HTTP error {status}")));
    }
    let json = JsFuture::from(resp.json().unwrap()).await.unwrap();
    Ok(json)
}
