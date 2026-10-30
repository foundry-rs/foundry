//! `forge script --arg NAME=VALUE`: arguments matched to the script function's parameter names.
//!
//! Missing arguments take the values of the function's `@custom:defaults` NatSpec tag, a list of
//! `name=value` pairs, e.g. `/// @custom:defaults capacity=1000000000000000000 rate=1000`.

use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::{Function, JsonAbi};
use eyre::{Result, bail, eyre};
use foundry_common::abi::coerce_value;
use std::{collections::BTreeMap, fmt::Write};

/// NatSpec of one function: `@param` descriptions and `@custom:defaults` values.
#[derive(Debug, Default)]
struct FunctionDocs {
    params: BTreeMap<String, String>,
    defaults: BTreeMap<String, String>,
}

impl FunctionDocs {
    /// Reads the docs of `func` from a contract's raw solc metadata.
    fn new(raw_metadata: Option<&str>, func: &Function) -> Result<Self> {
        let Some(raw) = raw_metadata else { return Ok(Self::default()) };
        let metadata: serde_json::Value = serde_json::from_str(raw)?;
        let method = &metadata["output"]["devdoc"]["methods"][func.signature()];
        let params = method["params"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(name, doc)| Some((name.clone(), doc.as_str()?.to_string())))
            .collect();
        let defaults = match method["custom:defaults"].as_str() {
            Some(defaults) => parse_defaults(defaults).map_err(|err| {
                eyre!("invalid `@custom:defaults` on `{}`: {err}", func.signature())
            })?,
            None => BTreeMap::new(),
        };
        Ok(Self { params, defaults })
    }
}

/// Parses whitespace-separated `name=value` pairs. Values with spaces are quoted.
fn parse_defaults(defaults: &str) -> Result<BTreeMap<String, String>> {
    let entries = shlex::split(defaults).ok_or_else(|| eyre!("unbalanced quotes"))?;
    parse_pairs(entries, "entry")
}

/// Parses `NAME=VALUE` pairs, rejecting duplicates.
fn parse_pairs(
    pairs: impl IntoIterator<Item = String>,
    what: &str,
) -> Result<BTreeMap<String, String>> {
    let mut parsed = BTreeMap::new();
    for pair in pairs {
        let (name, value) =
            pair.split_once('=').ok_or_else(|| eyre!("{what} `{pair}` is not NAME=VALUE"))?;
        if parsed.insert(name.to_string(), value.to_string()).is_some() {
            bail!("{what} `{name}` is given twice");
        }
    }
    Ok(parsed)
}

/// Returns the function `sig` (a name or a full signature) and its calldata, with arguments given
/// by parameter name.
///
/// With overloads, the function whose parameters fit the given names, with defaults for the rest,
/// is chosen.
pub(crate) fn method_and_calldata(
    sig: &str,
    abi: &JsonAbi,
    raw_metadata: Option<&str>,
    named_args: &[String],
) -> Result<(Function, Vec<u8>)> {
    let given = parse_pairs(named_args.iter().cloned(), "argument")?;
    let candidates = abi
        .functions()
        .filter(|func| if sig.contains('(') { func.signature() == sig } else { func.name == sig })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        bail!("Function `{sig}` not found in the ABI");
    }

    let mut fitting = Vec::new();
    let mut errors = Vec::new();
    for func in candidates {
        let docs = FunctionDocs::new(raw_metadata, func)?;
        match ordered_values(func, &docs, &given) {
            Ok(values) => fitting.push((func, values)),
            Err(err) => errors.push(err),
        }
    }
    match fitting.len() {
        0 => Err(eyre!("{}", errors.join("\n\n"))),
        1 => {
            let (func, values) = fitting.remove(0);
            // Same parsing as positional arguments, with errors naming the argument.
            let values = std::iter::zip(&func.inputs, &values)
                .map(|(input, value)| {
                    let ty = input.selector_type();
                    coerce_value(&ty, value)
                        .map_err(|err| eyre!("argument `{}` ({ty}): {err}", input.name))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((func.clone(), func.abi_encode_input(&values)?))
        }
        _ => bail!(
            "the arguments fit several functions: {}; pass the full signature with --sig",
            fitting.iter().map(|(func, _)| func.signature()).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Returns the argument values of `func` in parameter order, or an error that lists its
/// parameters.
fn ordered_values(
    func: &Function,
    docs: &FunctionDocs,
    given: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    for name in given.keys() {
        if !func.inputs.iter().any(|input| input.name == *name) {
            problems.push(format!("unknown argument `{name}`"));
        }
    }
    let mut values = Vec::with_capacity(func.inputs.len());
    for (i, input) in func.inputs.iter().enumerate() {
        if input.name.is_empty() {
            problems.push(format!("parameter {i} has no name; pass positional arguments instead"));
            continue;
        }
        match given.get(&input.name).or_else(|| docs.defaults.get(&input.name)) {
            Some(value) => values.push(value.clone()),
            None => problems.push(format!("missing argument `{}`", input.name)),
        }
    }
    if problems.is_empty() {
        return Ok(values);
    }

    let mut message = format!("{} for {}:", problems.join(", "), func.signature());
    for input in &func.inputs {
        let _ = write!(message, "\n  {}  {}", input.name, input.ty);
        if let Some(doc) = docs.params.get(&input.name) {
            let _ = write!(message, "  {doc}");
        }
        if let Some(default) = docs.defaults.get(&input.name) {
            let _ = write!(message, " (default {default})");
        }
    }
    Err(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_common::abi::encode_function_args;

    fn abi(sigs: &[&str]) -> JsonAbi {
        let sigs = sigs.iter().map(|sig| format!("function {sig}")).collect::<Vec<_>>();
        JsonAbi::parse(sigs.iter().map(String::as_str)).unwrap()
    }

    fn metadata(sig: &str, defaults: &str) -> String {
        serde_json::json!({ "output": { "devdoc": { "methods": { sig: {
            "custom:defaults": defaults,
            "params": { "capacity": "Rate limit capacity in wei" },
        } } } } })
        .to_string()
    }

    fn args(args: &[&str]) -> Vec<String> {
        args.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn named_arguments_in_any_order_with_defaults() {
        let abi = abi(&["run(string chain, uint256 capacity, uint256 rate)"]);
        let meta = metadata("run(string,uint256,uint256)", "capacity=1000 rate=7");
        let (func, data) =
            method_and_calldata("run", &abi, Some(&meta), &args(&["rate=9", "chain=sepolia"]))
                .unwrap();
        let expected =
            encode_function_args(func_ref(&abi, "run"), ["sepolia", "1000", "9"]).unwrap();
        assert_eq!(func.signature(), "run(string,uint256,uint256)");
        assert_eq!(data, expected);
    }

    fn func_ref<'a>(abi: &'a JsonAbi, name: &str) -> &'a Function {
        abi.functions().find(|func| func.name == name).unwrap()
    }

    #[test]
    fn missing_and_unknown_arguments_list_the_parameters() {
        let abi = abi(&["run(string chain, uint256 capacity)"]);
        let meta = metadata("run(string,uint256)", "capacity=1000");
        let err = method_and_calldata("run", &abi, Some(&meta), &args(&["chian=sepolia"]))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "unknown argument `chian`, missing argument `chain` for run(string,uint256):\n  \
             chain  string\n  capacity  uint256  Rate limit capacity in wei (default 1000)"
        );
    }

    #[test]
    fn overloads_are_chosen_by_parameter_names() {
        let abi = abi(&["run(string chain)", "run(string chain, uint256 capacity)"]);
        let (func, _) =
            method_and_calldata("run", &abi, None, &args(&["chain=a", "capacity=1"])).unwrap();
        assert_eq!(func.signature(), "run(string,uint256)");
        let (func, _) = method_and_calldata("run", &abi, None, &args(&["chain=a"])).unwrap();
        assert_eq!(func.signature(), "run(string)");
    }

    #[test]
    fn arrays_structs_and_other_types() {
        // `Lane` is a struct, i.e. a tuple parameter; `lanes` an array of structs.
        let abi: JsonAbi = serde_json::from_str(
            r#"[{"type":"function","name":"run","stateMutability":"nonpayable","outputs":[],
            "inputs":[
              {"name":"recipients","type":"address[]"},
              {"name":"lane","type":"tuple","internalType":"struct Lane","components":[
                {"name":"selector","type":"uint64"},{"name":"pool","type":"address"}]},
              {"name":"lanes","type":"tuple[]","internalType":"struct Lane[]","components":[
                {"name":"selector","type":"uint64"},{"name":"pool","type":"address"}]},
              {"name":"data","type":"bytes"},
              {"name":"enabled","type":"bool"},
              {"name":"label","type":"string"}]}]"#,
        )
        .unwrap();
        let meta = serde_json::json!({ "output": { "devdoc": { "methods": {
            "run(address[],(uint64,address),(uint64,address)[],bytes,bool,string)": {
                "custom:defaults": r#"enabled=true label="two words" lanes=[(1,0x0000000000000000000000000000000000000002)]"#,
            }
        } } } })
        .to_string();
        let (func, data) = method_and_calldata(
            "run",
            &abi,
            Some(&meta),
            &args(&[
                "recipients=[0x0000000000000000000000000000000000000001,0x0000000000000000000000000000000000000002]",
                "lane=(16015286601757825753,0x0000000000000000000000000000000000000003)",
                "data=0xdeadbeef",
            ]),
        )
        .unwrap();

        // Same calldata as the positional arguments in declaration order.
        let positional = encode_function_args(
            func_ref(&abi, "run"),
            [
                "[0x0000000000000000000000000000000000000001,0x0000000000000000000000000000000000000002]",
                "(16015286601757825753,0x0000000000000000000000000000000000000003)",
                "[(1,0x0000000000000000000000000000000000000002)]",
                "0xdeadbeef",
                "true",
                "two words",
            ],
        )
        .unwrap();
        assert_eq!(
            func.signature(),
            "run(address[],(uint64,address),(uint64,address)[],bytes,bool,string)"
        );
        assert_eq!(data, positional);
    }

    #[test]
    fn invalid_values_name_the_argument() {
        let abi = abi(&["run(uint256[3] amounts, uint8 small)"]);
        let err = method_and_calldata("run", &abi, None, &args(&["amounts=[1,2,3]", "small=256"]))
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("argument `small` (uint8): "), "{err}");
        let err = method_and_calldata("run", &abi, None, &args(&["amounts=[1,2]", "small=1"]))
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("argument `amounts` (uint256[3]): "), "{err}");
    }

    #[test]
    fn rejects_duplicates_and_malformed_pairs() {
        let abi = abi(&["run(uint256 a)"]);
        let err = method_and_calldata("run", &abi, None, &args(&["a=1", "a=2"])).unwrap_err();
        assert_eq!(err.to_string(), "argument `a` is given twice");
        let err = method_and_calldata("run", &abi, None, &args(&["a"])).unwrap_err();
        assert_eq!(err.to_string(), "argument `a` is not NAME=VALUE");
        assert_eq!(
            parse_defaults(r#"name="My Token" cap=5"#).unwrap(),
            BTreeMap::from([("cap".into(), "5".into()), ("name".into(), "My Token".into())])
        );
    }
}
