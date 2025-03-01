use crate::{
    evm::{
        element::Element,
        op,
        vm::{StepResult, Vm},
        U256, VAL_0_B, VAL_1, VAL_1_B, VAL_32_B,
    },
    utils::{execute_until_function_start, and_mask_to_type},
    Selector,
};
use alloy_dyn_abi::DynSolType;
use alloy_primitives::uint;
use std::{
    cmp::max,
    collections::{BTreeMap, BTreeSet},
};

mod calldata;
use calldata::CallDataImpl;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Label {
    CallData,
    Arg(Val),
    IsZeroResult(Val),
}

const VAL_2: U256 = uint!(2_U256);
const VAL_31_B: [u8; 32] = uint!(31_U256).to_be_bytes();

#[derive(Clone, Debug, PartialEq, Eq)]
struct Val {
    offset: u32,
    path: Vec<u32>,
    add_val: u32,
    and_mask: Option<U256>,
}

#[derive(PartialEq, Debug)]
enum InfoVal {
    // (x) - number of elements
    Dynamic(u32), // string|bytes|tuple|array
    Array(u32),
}

#[derive(Default, Debug)]
struct Info {
    tinfo: Option<InfoVal>,
    tname: Option<(DynSolType, u8)>,
    children: BTreeMap<u32, Info>,
}

impl Info {
    fn new() -> Self {
        Self {
            tinfo: None,
            tname: None,
            children: BTreeMap::new(),
        }
    }

    fn to_alloy_type(&self, is_root: bool) -> Vec<DynSolType> {
        if let Some((name, _)) = &self.tname {
            if matches!(name, DynSolType::Bytes) {
                if let Some(InfoVal::Array(0)) | Some(InfoVal::Dynamic(1)) | None = self.tinfo {
                    return vec![name.clone()];
                }
            } else if self.children.is_empty() {
                if let Some(InfoVal::Dynamic(_)) | None = self.tinfo {
                    return vec![name.clone()];
                }
            }
        }

        let start_key = if let Some(InfoVal::Array(_)) = self.tinfo {
            32
        } else {
            0
        };
        let mut end_key = if let Some((k, _)) = self.children.last_key_value() {
            *k
        } else {
            0
        };
        if let Some(InfoVal::Array(n_elements) | InfoVal::Dynamic(n_elements)) = self.tinfo {
            end_key = max(end_key, n_elements * 32);
        }

        let q: Vec<_> = (start_key..=end_key)
            .step_by(32)
            .flat_map(|k| {
                self.children
                    .get(&k)
                    .map_or(vec![DynSolType::Uint(256)], |val| val.to_alloy_type(false))
                    .into_iter()
            })
            .collect();

        let c = if q.len() > 1 && !is_root {
            vec![DynSolType::Tuple(q.clone())]
        } else {
            q.clone()
        };

        match self.tinfo {
            Some(InfoVal::Array(_)) => {
                vec![if q.len() == 1 {
                    DynSolType::Array(Box::new(q[0].clone()))
                } else {
                    DynSolType::Array(Box::new(DynSolType::Tuple(q)))
                }]
            }
            Some(InfoVal::Dynamic(_)) => {
                if end_key == 0 && self.children.is_empty() {
                    return vec![DynSolType::Bytes];
                }
                if end_key == 32 {
                    if self.children.is_empty() {
                        return vec![DynSolType::Array(Box::new(DynSolType::Uint(256)))];
                    }
                    if self.children.len() == 1
                        && self.children.first_key_value().expect("len checked above").1.tinfo.is_none()
                    {
                        return vec![DynSolType::Array(Box::new(q[1].clone()))];
                    }
                }
                c
            }
            None => c,
        }
    }
}

#[derive(Debug)]
struct ArgsResult {
    data: Info,
    not_bool: BTreeSet<Vec<u32>>,
}

impl ArgsResult {
    fn new() -> Self {
        Self {
            data: Info::new(),
            not_bool: BTreeSet::new(),
        }
    }

    fn get_or_create(&mut self, path: &[u32]) -> &mut Info {
        path.iter().fold(&mut self.data, |node, &key| {
            node.children.entry(key).or_default()
        })
    }

    fn get_mut(&mut self, path: &[u32]) -> Option<&mut Info> {
        path.iter()
            .try_fold(&mut self.data, |node, &key| node.children.get_mut(&key))
    }

    fn mark_not_bool(&mut self, path: &[u32], offset: u32) {
        let full_path = [path, &[offset]].concat();

        if let Some(el) = self.get_mut(&full_path) {
            if let Some((v, _)) = &mut el.tname {
                if matches!(v, DynSolType::Bool) {
                    el.tname = None;
                }
            }
        }

        self.not_bool.insert(full_path);
    }

    fn set_tname(&mut self, path: &[u32], offset: u32, tname: DynSolType, confidence: u8) {
        let full_path = [path, &[offset]].concat();

        if matches!(tname, DynSolType::Bool) && self.not_bool.contains(&full_path) {
            return;
        }

        let el = self.get_or_create(&full_path);
        if let Some((_, conf)) = el.tname {
            if confidence <= conf {
                return;
            }
        }
        el.tname = Some((tname, confidence));
    }

    fn array_in_path(&self, path: &[u32]) -> Vec<bool> {
        path.iter()
            .scan(&self.data, |el, &p| {
                *el = el.children.get(&p)?;
                Some(matches!(el.tinfo, Some(InfoVal::Array(_))))
            })
            .collect()
    }

    fn set_info(&mut self, path: &[u32], tinfo: InfoVal) {
        if path.is_empty() { // root
            return;
        }
        let el = self.get_or_create(path);

        if let InfoVal::Dynamic(n) = tinfo {
            match el.tinfo {
                Some(InfoVal::Dynamic(x)) => {
                    if x > n {
                        return;
                    }
                }
                Some(InfoVal::Array(_)) => return,
                None => (),
            };
        }

        if let Some(InfoVal::Array(p)) = el.tinfo {
            if let InfoVal::Array(n) = tinfo {
                if n < p {
                    return;
                }
            };
        }
        el.tinfo = Some(tinfo);
    }
}

fn analyze(
    vm: &mut Vm<Label, CallDataImpl>,
    args: &mut ArgsResult,
    ret: StepResult<Label>,
) -> Result<(), Box<dyn std::error::Error>> {
    match ret {
        StepResult{op: op @ (op::CALLDATALOAD | op::CALLDATACOPY),  fa: Some(Element{label: Some(Label::Arg(Val{offset, path, add_val, ..})), ..}), sa, ..} =>
        {
            if add_val >= 4 && (add_val - 4) % 32 == 0 {
                let mut full_path = path.clone();
                full_path.push(offset);

                let mut po: u32 = 0;
                if add_val != 4 {
                    po += args
                        .array_in_path(&path)
                        .iter()
                        .fold(0, |s, &is_arr| if is_arr { s + 32 } else { s });
                    if po > (add_val - 4) {
                        po = 0;
                    }
                }

                let new_off = add_val - 4 - po;

                args.set_info(&full_path, InfoVal::Dynamic(new_off / 32));

                let mem_offset = if op == op::CALLDATACOPY {
                    u32::try_from(sa.expect("always set for DATACOPY in vm.rs"))
                        .expect("set as u32 in vm.rs")
                } else {
                    0
                };

                if new_off == 0 && *args.array_in_path(&full_path).last().unwrap_or(&false) {
                    match op {
                        op::CALLDATALOAD => vm.stack.peek_mut()?.data = VAL_1_B,
                        op::CALLDATACOPY => {
                            if let Some(v) = vm.memory.get_mut(mem_offset) {
                                v.data = VAL_1_B.to_vec();
                            }
                        }
                        _ => (),
                    }
                }

                let new_label = Some(Label::Arg(Val {
                    offset: new_off,
                    path: full_path,
                    add_val: 0,
                    and_mask: None,
                }));
                match op {
                    op::CALLDATALOAD => vm.stack.peek_mut()?.label = new_label,
                    op::CALLDATACOPY => {
                        if let Some(v) = vm.memory.get_mut(mem_offset) {
                            args.set_tname(&path, offset, DynSolType::Bytes, 10);
                            v.label = new_label;
                        }
                    }
                    _ => (),
                }
            }
        }

        StepResult{op: op @ (op::CALLDATALOAD | op::CALLDATACOPY), fa: Some(el), sa, ..} =>
        {
            if let Ok(off) = u32::try_from(el) {
                if (4..131072 - 1024).contains(&off) {
                    // 131072 is constant from ./calldata.rs
                    // -1024: cut 'trustedForwarder'
                    args.get_or_create(&[off - 4]);

                    let new_label = Some(Label::Arg(Val {
                        offset: off - 4,
                        path: Vec::new(),
                        add_val: 0,
                        and_mask: None,
                    }));
                    match op {
                        op::CALLDATALOAD => vm.stack.peek_mut()?.label = new_label,
                        op::CALLDATACOPY => {
                            let mem_offset =
                                u32::try_from(sa.expect("always set for DATACOPY in vm.rs"))
                                    .expect("set as u32 in vm.rs");
                            if let Some(v) = vm.memory.get_mut(mem_offset) {
                                v.label = new_label;
                            }
                        }
                        _ => (),
                    }
                }
            }
        }

        StepResult{
            op: op::ADD,
            fa: Some(Element{label: Some(Label::Arg(Val{offset: f_offset, path: f_path, add_val: f_add_val, and_mask: f_and_mask})), ..}),
            sa: Some(Element{label: Some(Label::Arg(Val{offset: s_offset, path: s_path, add_val: s_add_val, and_mask: s_and_mask})), ..}),
            ..} =>
        {
            args.mark_not_bool(&f_path, f_offset);
            args.mark_not_bool(&s_path, s_offset);
            vm.stack.peek_mut()?.label = Some(Label::Arg(if f_path.len() > s_path.len() {
                Val {
                    offset: f_offset,
                    path: f_path,
                    add_val: f_add_val + s_add_val,
                    and_mask: f_and_mask,
                }
            } else {
                Val {
                    offset: s_offset,
                    path: s_path,
                    add_val: s_add_val + f_add_val,
                    and_mask: s_and_mask,
                }
            }));
        }

          StepResult{op: op::ADD, fa: Some(Element{label: Some(Label::Arg(Val{offset, path, add_val, and_mask})), data, ..}), sa: Some(ot), ..}
        | StepResult{op: op::ADD, sa: Some(Element{label: Some(Label::Arg(Val{offset, path, add_val, and_mask})), data, ..}), fa: Some(ot), ..} =>
        {
            args.mark_not_bool(&path, offset);
            if offset == 0
                && add_val == 0
                && !path.is_empty()
                && data == VAL_0_B
                && ot.data == U256::MAX.to_be_bytes()
            {
                vm.stack.peek_mut()?.data = VAL_0_B; // sub(-1) as add(0xff..ff)
            }
            if let Ok(val) = u32::try_from(U256::from_be_bytes(ot.data) + U256::from(add_val)) {
                vm.stack.peek_mut()?.label = Some(Label::Arg(Val {
                    offset,
                    path,
                    add_val: val,
                    and_mask,
                }));
            }
        }

          StepResult{op: op @ op::MUL, fa: Some(Element{label: Some(Label::Arg(Val{offset: 0, path, add_val: 0, ..})), ..}), sa: Some(ot), ..}
        | StepResult{op: op @ op::MUL, sa: Some(Element{label: Some(Label::Arg(Val{offset: 0, path, add_val: 0, ..})), ..}), fa: Some(ot), ..}
        | StepResult{op: op @ op::SHL, sa: Some(Element{label: Some(Label::Arg(Val{offset: 0, path, add_val: 0, ..})), ..}), fa: Some(ot), ..} =>
        {
            args.mark_not_bool(&path, 0);
            if let Some(Label::Arg(Val {
                offset: o1,
                path: ref p1,
                ..
            })) = ot.label
            {
                args.mark_not_bool(p1, o1);
            }
            if !path.is_empty() {
                let mut mult: U256 = ot.into();
                if op == op::SHL {
                    mult = VAL_1 << mult;
                }

                match mult {
                    VAL_1 => {
                        if let Some((last, rest)) = path.split_last() {
                            args.set_tname(rest, *last, DynSolType::Bytes, 10);
                        }
                    }

                    VAL_2 => {
                        // slen*2+1 for SSTORE
                        if let Some((last, rest)) = path.split_last() {
                            args.set_tname(rest, *last, DynSolType::String, 20);
                        }
                    }

                    _ => {
                        let otr: Result<u32, _> = mult.try_into();
                        if let Ok(m) = otr {
                            if m % 32 == 0 && (32..3200).contains(&m) {
                                args.set_info(&path, InfoVal::Array(m / 32));

                                for el in vm.stack.data.iter_mut() {
                                    if let Some(Label::Arg(lab)) = &el.label {
                                        if lab.offset == 0 && lab.path == path && lab.add_val == 0 {
                                            el.data = VAL_1_B;
                                        }
                                    }
                                }

                                for el in vm.memory.data.iter_mut() {
                                    if let Some(Label::Arg(lab)) = &el.1.label {
                                        if lab.offset == 0 && lab.path == path && lab.add_val == 0 {
                                            el.1.data = VAL_1_B.to_vec();
                                        }
                                    }
                                }

                                // simulate arglen = 1
                                vm.stack.peek_mut()?.data = mult.to_be_bytes();
                            }
                        }
                    }
                }
            }
        }

        // 0 < arr.len || arr.len > 0
          StepResult{op: op::LT, sa: Some(Element{label: Some(Label::Arg(Val{offset: 0, path, add_val: 0, and_mask: None})), ..}), fa: Some(ot), ..}
        | StepResult{op: op::GT, fa: Some(Element{label: Some(Label::Arg(Val{offset: 0, path, add_val: 0, and_mask: None})), ..}), sa: Some(ot), ..} =>
        {
            args.mark_not_bool(&path, 0);
            // 31 = string for storage
            if ot.data == VAL_0_B || ot.data == VAL_31_B {
                vm.stack.peek_mut()?.data = VAL_1_B;
            }
        }

          StepResult{op: op::LT|op::GT|op::MUL, fa: Some(Element{label: Some(Label::Arg(Val{offset, path, ..})), ..}), ..}
        | StepResult{op: op::LT|op::GT|op::MUL, sa: Some(Element{label: Some(Label::Arg(Val{offset, path, ..})), ..}), ..} =>
        {
            args.mark_not_bool(&path, offset);
        }

          StepResult{op: op::AND, fa: Some(Element{label: Some(Label::Arg(Val{offset, path, add_val, and_mask: None})), ..}), sa: Some(ot), ..}
        | StepResult{op: op::AND, sa: Some(Element{label: Some(Label::Arg(Val{offset, path, add_val, and_mask: None})), ..}), fa: Some(ot), ..} =>
        {
            args.mark_not_bool(&path, offset);
            let mask: U256 = ot.into();
            if let Some(t) = and_mask_to_type(mask) {
                args.set_tname(&path, offset, t, 5);
                vm.stack.peek_mut()?.label = Some(Label::Arg(Val {
                    offset,
                    path,
                    add_val,
                    and_mask: Some(mask),
                }));
            }
        }

          StepResult{op: op::EQ,
            fa: Some(Element{label: Some(Label::Arg(Val{offset, path, add_val, and_mask: None})), ..}),
            sa: Some(Element{label: Some(Label::Arg(Val{offset: s_offset, path: s_path, add_val: s_add_val, and_mask: Some(mask)})), ..}),
        ..} |
          StepResult{op: op::EQ,
            sa: Some(Element{label: Some(Label::Arg(Val{offset, path, add_val, and_mask: None})), ..}),
            fa: Some(Element{label: Some(Label::Arg(Val{offset: s_offset, path: s_path, add_val: s_add_val, and_mask: Some(mask)})), ..}),
        ..} =>
        {
            if (s_offset == offset) && (s_path == path) && (s_add_val == add_val) {
                if let Some(t) = and_mask_to_type(mask) {
                    args.set_tname(&path, offset, t, 20);
                }
            }
        }

        StepResult{op: op::ISZERO, fa: Some(Element{label: Some(Label::Arg(val)), ..}), ..} =>
        {
            vm.stack.peek_mut()?.label = Some(Label::IsZeroResult(val));
        }

        StepResult{op: op::ISZERO, fa: Some(Element{label: Some(Label::IsZeroResult(val)), ..}), ..} =>
        {
            // Detect check for 0 in DIV, it's not bool in that case: ISZERO, ISZERO, PUSH off, JUMPI, JUMPDEST, DIV
            // for solidity < 0.6.0
            let mut is_bool = true;
            let op = vm.code[vm.pc];
            if let op::PUSH1..=op::PUSH4 = op {
                let n = (op - op::PUSH0) as usize;
                if vm.code[vm.pc + n + 1] == op::JUMPI {
                    let mut arg: [u8; 4] = [0; 4];
                    arg[(4 - n)..].copy_from_slice(&vm.code[vm.pc + 1..vm.pc + 1 + n]);
                    let jumpdest = u32::from_be_bytes(arg) as usize;
                    if jumpdest + 1 < vm.code.len()
                        && vm.code[jumpdest] == op::JUMPDEST
                        && vm.code[jumpdest + 1] == op::DIV
                    {
                        is_bool = false;
                    }
                }
            }

            if is_bool {
                args.set_tname(&val.path, val.offset, DynSolType::Bool, 5);
            }
        }

        StepResult{op: op::SIGNEXTEND, sa: Some(Element{label: Some(Label::Arg(Val{offset, path, ..})), ..}), fa: Some(s0), ..} =>
        {
            if s0.data < VAL_32_B {
                let s0: u8 = s0.data[31];
                args.set_tname(&path, offset, DynSolType::Int((s0 as usize + 1) * 8), 20);
            }
        }

        StepResult{op: op::BYTE, sa: Some(Element{label: Some(Label::Arg(Val{offset, path, ..})), ..}), ..} =>
        {
            args.set_tname(&path, offset, DynSolType::FixedBytes(32), 4);
        }

        _ => (),
    };
    Ok(())
}

/// Extracts function arguments and returns them as Alloy types
///
/// # Arguments
///
/// * `code` - A slice of deployed contract bytecode
/// * `selector` - A function selector
/// * `gas_limit` - Maximum allowed gas usage; set to `0` to use defaults
///
/// # Examples
///
/// ```
/// use evmole::function_arguments_alloy;
/// use alloy_primitives::hex;
/// use alloy_dyn_abi::DynSolType;
///
/// let code = hex::decode("6080604052348015600e575f80fd5b50600436106030575f3560e01c80632125b65b146034578063b69ef8a8146044575b5f80fd5b6044603f3660046046565b505050565b005b5f805f606084860312156057575f80fd5b833563ffffffff811681146069575f80fd5b925060208401356001600160a01b03811681146083575f80fd5b915060408401356001600160e01b0381168114609d575f80fd5b80915050925092509256").unwrap();
/// let selector = [0x21, 0x25, 0xb6, 0x5b];
///
/// let arguments: Vec<DynSolType> = function_arguments_alloy(&code, &selector, 0);
///
/// assert_eq!(arguments, vec![DynSolType::Uint(32), DynSolType::Address, DynSolType::Uint(224)]);
/// ```
#[deprecated(since = "0.6.0", note = "Use contract_info(ContractInfoArgs(code).with_arguments()) instead")]
pub fn function_arguments_alloy(
    code: &[u8],
    selector: &Selector,
    gas_limit: u32,
) -> Vec<DynSolType> {
    if cfg!(feature = "trace_arguments") {
        println!(
            "Processing selector {:02x}{:02x}{:02x}{:02x}",
            selector[0], selector[1], selector[2], selector[3]
        );
    }
    let calldata = CallDataImpl { selector: *selector };
    let mut vm = Vm::new(code, &calldata);
    let mut args = ArgsResult::new();
    let mut gas_used = 0;
    let real_gas_limit = if gas_limit == 0 {
        5e4 as u32
    } else {
        gas_limit
    };

    if let Some(g) = execute_until_function_start(&mut vm, real_gas_limit) {
        gas_used += g;
    } else {
        return vec![];
    }

    while !vm.stopped {
        if cfg!(feature = "trace_arguments") {
            println!("args: {:?}", args);
            println!("not_bool: {:?}", args.not_bool);
            println!("{:#?}", args.data);
            println!("{:?}\n", vm);
        }
        let ret = match vm.step() {
            Ok(v) => v,
            Err(_e) => {
                // println!("{}", _e);
                break;
            }
        };
        gas_used += ret.gas_used;
        if gas_used > real_gas_limit {
            break;
        }

        if analyze(&mut vm, &mut args, ret).is_err() {
            break;
        }
    }

    if args.data.children.is_empty() {
        vec![]
    } else {
        args.data.to_alloy_type(true)
    }
}

/// Extracts function arguments and returns them as a string
///
/// # Arguments
///
/// * `code` - A slice of deployed contract bytecode
/// * `selector` - A function selector
/// * `gas_limit` - Maximum allowed gas usage; set to `0` to use defaults
///
/// # Examples
///
/// ```
/// use evmole::function_arguments;
/// use alloy_primitives::hex;
///
/// let code = hex::decode("6080604052348015600e575f80fd5b50600436106030575f3560e01c80632125b65b146034578063b69ef8a8146044575b5f80fd5b6044603f3660046046565b505050565b005b5f805f606084860312156057575f80fd5b833563ffffffff811681146069575f80fd5b925060208401356001600160a01b03811681146083575f80fd5b915060408401356001600160e01b0381168114609d575f80fd5b80915050925092509256").unwrap();
/// let selector = [0x21, 0x25, 0xb6, 0x5b];
///
/// let arguments: String = function_arguments(&code, &selector, 0);
///
/// assert_eq!(arguments, "uint32,address,uint224");
/// ```
#[deprecated(since = "0.6.0", note = "Use contract_info(ContractInfoArgs(code).with_arguments()) instead")]
pub fn function_arguments(code: &[u8], selector: &Selector, gas_limit: u32) -> String {
    #[allow(deprecated)]
    function_arguments_alloy(code, selector, gas_limit)
        .into_iter()
        .map(|t| t.sol_type_name().to_string())
        .collect::<Vec<String>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::function_arguments;
    use crate::function_selectors;
    use alloy_primitives::hex;

    #[test]
    fn test_code_offset_buffer() {
        // This is a solidity trick to quickly zeroize memory, but
        // causing crashes in earlier evmole implementation
        //
        // Mainnet 0x27e70bfdf7de32bae2274c8d37d51934ff098910
        let code = hex::decode("6000608052600060a052600060c052600060e05260006101005260006101205260006101405260006101605260006101805260006101a05260006101c05260006101e05260006102005260006102205260006102405260006102605260006102805260006102a05260006102c05260006102e05260006103005260006103205260006103405260006103605260006103805260006103a05260006103c05260006103e0526000610400526000610420526000610440527f66702d66702d7075662d763100000000000000000000000000000000000000006000554360018060000101556101806103dc610200396102005160045561022051600855610240516006556102605160075561028051600a556102a051600b556102c0516001556102e0516010556103005160115561032051601455610340516080906103dc9060208101101561014c57600080fd5b602061034051016103dc01101561016257600080fd5b6103405160208101101561017557600080fd5b602061034051016103dc0161038039610380516012556103a0516013556103c0516015556103e051601655610360516020906103dc90810110156101b857600080fd5b610360516103dc016104003961040051600c556103dc610240810110156101de57600080fd5b6102406103dc016103605260006104605261040051610480525b610480511561033157602061036051600160006104605114610218575060005b6102405760206104605160206104605102041461023457600080fd5b60206104605102610243565b60005b6103605101101561025357600080fd5b600160006104605114610264575060005b61028c5760206104605160206104605102041461028057600080fd5b6020610460510261028f565b60005b6103605101610420396104205161044051810110156102ad57600080fd5b610440516104205101610440527f7061796d656e740000000000000000000000000000000000000000000000000060c0526104605160e05261042051604060c020556104605160016104605101101561030557600080fd5b6001610460510161046052610480516001111561032157600080fd5b60016104805103610480526101f8565b341561033c57600080fd5b60016003557f587ece4cd19692c5be1a4184503d607d45542d2aca0698c0068f52e09ccb541c6040610200a16066806103766000396000f3007c010000000000000000000000000000000000000000000000000000000060003504608081905263696eb8fb1415603e576000546104a0908152602090f35b366000803760008036600060016000015460155a03f4605c57600080fd5b3d6000803e3d6000f3").unwrap();

        for sig in function_selectors(&code, 0) {
            let _ = function_arguments(&code, &sig, 0);
        }
    }
}
