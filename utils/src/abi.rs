//! Convert a json abi into solidity inerface

use ethers_contract::InternalStructs;
use ethers_core::{
    abi,
    abi::{
        struct_def::{FieldType, StructFieldType},
        Contract as Abi, Event, EventParam, Function, Param, ParamType, RawAbi, SolStruct,
    },
};
use std::collections::BTreeMap;

/// This function takes a contract [`Abi`] and a name and proceeds to generate a Solidity
/// `interface` from that ABI. If the provided name is empty, then it defaults to `interface
/// Interface`.
///
/// This is done by iterating over the functions and their ABI inputs/outputs, and generating
/// function signatures/inputs/outputs according to the ABI.
///
/// Notes:
/// * ABI Encoder V2 is not supported yet
/// * Kudos to [maxme/abi2solidity](https://github.com/maxme/abi2solidity) for the algorithm
///
/// Note: This takes a raw representation of the json abi (`RawAbi`) because the `ethers::abi::Abi`
/// does not deserialize the internal type of nested params which is required in order to generate
/// structs
pub fn abi_to_solidity(contract_abi: &RawAbi, mut contract_name: &str) -> eyre::Result<String> {
    if contract_name.trim().is_empty() {
        contract_name = "Interface";
    };

    let structs = InternalStructs::new(contract_abi.clone());

    // this is a bit horrible but the easiest way to convert the types
    let abi_str = serde_json::to_string(contract_abi)?;
    let contract_abi: Abi = serde_json::from_str(&abi_str)?;

    let mut events = Vec::with_capacity(contract_abi.events.len());
    for event in contract_abi.events() {
        let inputs = event
            .inputs
            .iter()
            .enumerate()
            .map(|(idx, param)| format_event_params(event, param, idx, &structs))
            .collect::<eyre::Result<Vec<String>>>()?
            .join(", ");

        let event_final = format!("event {}({inputs})", event.name);

        events.push(format!("{event_final};"));
    }

    let mut functions = Vec::with_capacity(contract_abi.functions.len());
    for function in contract_abi.functions() {
        let inputs = function
            .inputs
            .iter()
            .map(|param| format_function_input_param(function, param, &structs))
            .collect::<eyre::Result<Vec<String>>>()?
            .join(", ");

        let outputs = function
            .outputs
            .iter()
            .map(|param| format_function_output_param(function, param, &structs))
            .collect::<eyre::Result<Vec<String>>>()?
            .join(", ");

        let mutability = match function.state_mutability {
            abi::StateMutability::Pure => "pure",
            abi::StateMutability::View => "view",
            abi::StateMutability::Payable => "payable",
            _ => "",
        };

        let mut func = format!("function {}({inputs})", function.name);
        if !mutability.is_empty() {
            func = format!("{func} {mutability}");
        }
        func = format!("{func} external");
        if !outputs.is_empty() {
            func = format!("{func} returns ({outputs})");
        }

        functions.push(format!("{func};"));
    }

    let functions = functions.join("\n");
    let events = events.join("\n");

    let sol = if structs.structs_types().is_empty() {
        if events.is_empty() {
            format!(
                r#"interface {contract_name} {{
    {functions}
}}
"#
            )
        } else {
            format!(
                r#"interface {contract_name} {{
    {events}

    {functions}
}}
"#
            )
        }
    } else {
        let structs = format_struct_types(&structs);
        match events.is_empty() {
            true => format!(
                r#"interface {contract_name} {{
    {structs}

    {functions}
}}
"#
            ),
            false => format!(
                r#"interface {contract_name} {{
    {events}

    {structs}

    {functions}
}}
"#
            ),
        }
    };
    forge_fmt::fmt(&sol).map_err(|err| eyre::eyre!(err.to_string()))
}

/// returns the Tokenstream for the corresponding rust type of the param
fn expand_input_param_type(
    fun: &Function,
    param: &str,
    kind: &ParamType,
    structs: &InternalStructs,
) -> eyre::Result<String> {
    match kind {
        ParamType::Array(ty) => {
            let ty = expand_input_param_type(fun, param, ty, structs)?;
            Ok(format!("{ty}[]"))
        }
        ParamType::FixedArray(ty, size) => {
            let ty = expand_input_param_type(fun, param, ty, structs)?;
            Ok(format!("{ty}[{}]", *size))
        }
        ParamType::Tuple(_) => {
            let ty = if let Some(struct_name) =
                structs.get_function_input_struct_solidity_id(&fun.name, param)
            {
                struct_name.rsplit('.').next().unwrap().to_string()
            } else {
                kind.to_string()
            };
            Ok(ty)
        }
        _ => Ok(kind.to_string()),
    }
}

fn expand_output_param_type(
    fun: &Function,
    param: &Param,
    kind: &ParamType,
    structs: &InternalStructs,
) -> eyre::Result<String> {
    match kind {
        ParamType::Array(ty) => {
            let ty = expand_output_param_type(fun, param, ty, structs)?;
            Ok(format!("{ty}[]"))
        }
        ParamType::FixedArray(ty, size) => {
            let ty = expand_output_param_type(fun, param, ty, structs)?;
            Ok(format!("{ty}[{}]", *size))
        }
        ParamType::Tuple(_) => {
            if param.internal_type.is_none() {
                let result =
                    kind.to_string().trim_start_matches('(').trim_end_matches(')').to_string();
                Ok(result)
            } else {
                let ty = if let Some(struct_name) = structs.get_function_output_struct_solidity_id(
                    &fun.name,
                    param.internal_type.as_ref().unwrap(),
                ) {
                    struct_name.rsplit('.').next().unwrap().to_string()
                } else {
                    kind.to_string()
                };
                Ok(ty)
            }
        }
        _ => Ok(kind.to_string()),
    }
}

// Returns the function parameter formatted as a string, as well as inserts into the provided
// `structs` set in order to create type definitions for any Abi Encoder v2 structs.
fn format_function_input_param(
    func: &Function,
    param: &Param,
    structs: &InternalStructs,
) -> eyre::Result<String> {
    let kind = expand_input_param_type(func, &param.name, &param.kind, structs)?;
    Ok(format_param(param, kind))
}

// Returns the function parameter formatted as a string, as well as inserts into the provided
// `structs` set in order to create type definitions for any Abi Encoder v2 structs.
fn format_function_output_param(
    func: &Function,
    param: &Param,
    structs: &InternalStructs,
) -> eyre::Result<String> {
    let kind = expand_output_param_type(func, param, &param.kind, structs)?;
    Ok(format_param(param, kind))
}

fn format_param(param: &Param, kind: String) -> String {
    // add `memory` if required (not needed for events, only for functions)
    let is_memory = match param.kind {
        ParamType::Array(_) |
        ParamType::Bytes |
        ParamType::String |
        ParamType::FixedArray(_, _) => true,
        ParamType::Tuple(_) => param.internal_type.is_some(),
        _ => false,
    };

    let kind = if is_memory { format!("{kind} memory") } else { kind };

    if param.name.is_empty() {
        kind
    } else {
        format!("{kind} {}", param.name)
    }
}

/// returns the Tokenstream for the corresponding rust type of the event_param
fn expand_event_param_type(
    event: &Event,
    kind: &ParamType,
    idx: usize,
    structs: &InternalStructs,
) -> eyre::Result<String> {
    match kind {
        ParamType::Array(ty) => {
            let ty = expand_event_param_type(event, ty, idx, structs)?;
            Ok(format!("{ty}[]"))
        }
        ParamType::FixedArray(ty, size) => {
            let ty = expand_event_param_type(event, ty, idx, structs)?;
            Ok(format!("{ty}[{}]", *size))
        }
        ParamType::Tuple(_) => {
            let ty = if let Some(struct_name) =
                structs.get_event_input_struct_solidity_id(&event.name, idx)
            {
                struct_name.rsplit('.').next().unwrap().to_string()
            } else {
                kind.to_string()
            };
            Ok(ty)
        }
        _ => Ok(kind.to_string()),
    }
}

fn format_event_params(
    event: &Event,
    param: &EventParam,
    idx: usize,
    structs: &InternalStructs,
) -> eyre::Result<String> {
    let kind = expand_event_param_type(event, &param.kind, idx, structs)?;
    let ty = if param.name.is_empty() {
        kind
    } else if param.indexed {
        format!("{kind} indexed {}", param.name)
    } else {
        format!("{kind} {}", param.name)
    };
    Ok(ty)
}

/// Returns all struct type defs
fn format_struct_types(structs: &InternalStructs) -> String {
    structs
        .structs_types()
        .iter()
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .map(|(name, ty)| format_struct_field(name, ty))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_struct_field(name: &str, sol_struct: &SolStruct) -> String {
    // strip member access if any
    let name = name.split('.').last().unwrap();
    let mut def = format!("struct {name} {{\n");
    for field in sol_struct.fields.iter() {
        let ty = match &field.ty {
            FieldType::Elementary(ty) => ty.to_string(),
            FieldType::Struct(ty) => struct_field_to_type(ty),
            FieldType::Mapping(_) => {
                unreachable!("illegal mapping type")
            }
        };

        def.push_str(&format!("{ty} {};\n", field.name));
    }

    def.push('}');

    def
}

fn struct_field_to_type(ty: &StructFieldType) -> String {
    match ty {
        StructFieldType::Type(ty) => ty.name().to_string(),
        StructFieldType::Array(ty) => {
            format!("{}[]", struct_field_to_type(ty))
        }
        StructFieldType::FixedArray(ty, size) => {
            format!("{}[{}]", struct_field_to_type(ty), *size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn abi2solidity() {
        let contract_abi: RawAbi = serde_json::from_str(include_str!(
            "../../testdata/fixtures/SolidityGeneration/InterfaceABI.json"
        ))
        .unwrap();
        pretty_assertions::assert_eq!(
            include_str!("../../testdata/fixtures/SolidityGeneration/GeneratedNamedInterface.sol"),
            abi_to_solidity(&contract_abi, "test").unwrap()
        );
        pretty_assertions::assert_eq!(
            include_str!(
                "../../testdata/fixtures/SolidityGeneration/GeneratedUnnamedInterface.sol"
            ),
            abi_to_solidity(&contract_abi, "").unwrap()
        );
    }
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn abi2solidity_gaugecontroller() {
        let contract_abi: RawAbi = serde_json::from_str(include_str!(
            "../../testdata/fixtures/SolidityGeneration/GaugeController.json"
        ))
        .unwrap();
        pretty_assertions::assert_eq!(
            include_str!("../../testdata/fixtures/SolidityGeneration/GeneratedGaugeController.sol"),
            abi_to_solidity(&contract_abi, "test").unwrap()
        );
    }
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn abi2dolidity_liquiditygauge() {
        let contract_abi: RawAbi = serde_json::from_str(include_str!(
            "../../testdata/fixtures/SolidityGeneration/LiquidityGaugeV4.json"
        ))
        .unwrap();
        pretty_assertions::assert_eq!(
            include_str!(
                "../../testdata/fixtures/SolidityGeneration/GeneratedLiquidityGaugeV4.sol"
            ),
            abi_to_solidity(&contract_abi, "test").unwrap()
        );
    }
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn abi2solidity_fastlane() {
        let contract_abi: RawAbi = serde_json::from_str(include_str!(
            "../../testdata/fixtures/SolidityGeneration/Fastlane.json"
        ))
        .unwrap();
        pretty_assertions::assert_eq!(
            include_str!("../../testdata/fixtures/SolidityGeneration/GeneratedFastLane.sol"),
            abi_to_solidity(&contract_abi, "test").unwrap()
        );
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn abi2solidity_with_structs() {
        let contract_abi: RawAbi = serde_json::from_str(include_str!(
            "../../testdata/fixtures/SolidityGeneration/WithStructs.json"
        ))
        .unwrap();
        pretty_assertions::assert_eq!(
            include_str!("../../testdata/fixtures/SolidityGeneration/WithStructs.sol").trim(),
            abi_to_solidity(&contract_abi, "test").unwrap().trim()
        );
    }
}
