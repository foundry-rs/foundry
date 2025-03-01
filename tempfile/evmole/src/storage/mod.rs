//! # Warning
//! This code is in an experimental state and under active development.
//! Code structure are subject to change.
use std::{cell::RefCell, collections::{BTreeMap, BTreeSet, HashMap}, rc::Rc};
use crate::{
    evm::{
        calldata::CallDataLabel, element::Element, op, vm::{StepResult, Vm}, U256, VAL_1, VAL_1_B, VAL_32_B
    }, utils::{and_mask_to_type, execute_until_function_start}, Selector, Slot,
};
use alloy_dyn_abi::DynSolType;

mod calldata;
use calldata::CallDataImpl;

mod keccak_precalc;
use keccak_precalc::KEC_PRECALC;


/// Represents a storage variable record in a smart contract's storage layout.
#[derive(Debug)]
pub struct StorageRecord {
    /// Storage slot location for the variable
    pub slot: Slot,

    /// Byte offset within the storage slot (0-31)
    pub offset: u8,

    /// Variable type
    pub r#type: String,

    /// Function selectors that read from this storage location
    pub reads: Vec<Selector>,

    /// Function selectors that write to this storage location
    pub writes: Vec<Selector>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Label {
    Constant,

    Typed(DynSolType),
    Sloaded(Rc<RefCell<StorageElement>>),
    IsZero(Rc<RefCell<StorageElement>>),
    Keccak(u32, Vec<Element<Label>>),
}

impl CallDataLabel for Label {
    fn label(_: usize, tp: &alloy_dyn_abi::DynSolType) -> Label {
        Label::Typed(tp.clone())
    }
}


fn get_base_internal_type(val: &DynSolType) -> DynSolType {
    if let DynSolType::Array(t) = val {
        get_base_internal_type(t)
    } else {
        val.clone()
    }
}

fn get_base_score(t: &DynSolType) -> usize {
    match t {
        DynSolType::Uint(256) => 1,
        DynSolType::Uint(8) => 3,
        DynSolType::Bool => 4,
        DynSolType::FixedBytes(32) => 6,
        DynSolType::FixedBytes(_) => 2,
        DynSolType::String | DynSolType::Bytes => 500,
        DynSolType::Array(v) => 5 * get_base_score(v),
        _ => 5,
    }
}


#[derive(Clone, PartialEq, Eq)]
enum StorageType {
    Base(DynSolType),
    Map(DynSolType, Box<StorageType>),
}

impl StorageType {
    fn set_type(&mut self, tp: DynSolType) {
        if let StorageType::Base(DynSolType::String) = self {
            return
        }
        match self {
            StorageType::Base(DynSolType::Array(ref mut v)) => {
                let mut current = v.as_mut();
                while let DynSolType::Array(inner) = current {
                    current = inner;
                    if let DynSolType::Uint(256) = &current {
                    }
                }
                *current = tp;
            }
            StorageType::Base(ref mut v) => *v = tp,
            StorageType::Map(_, ref mut v) => v.set_type(tp),
        }
    }


    fn get_internal_type(&self) -> DynSolType {
        match self {
        StorageType::Base(t) => get_base_internal_type(t),
        StorageType::Map(_, v) => v.get_internal_type(),
        }
    }


    fn get_score(&self) -> usize {
        match self {
            StorageType::Base(t) => get_base_score(t),
            StorageType::Map(k, v) => 1000 * get_base_score(k) + v.get_score(),
        }
    }
}

impl std::fmt::Debug for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageType::Base(v) => write!(f, "{}", v.sol_type_name()),
            StorageType::Map(k, v) => write!(f, "mapping({} => {:?})", k.sol_type_name(), v),
        }
    }
}


#[derive(Clone, PartialEq, Eq)]
struct StorageElement {
    slot: Slot,
    stype: StorageType,
    rshift: u8, // in bytes
    is_write: bool,
    last_and: Option<U256>,
    last_or2: Option<Element<Label>>,
}
impl std::fmt::Debug for StorageElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{:?}:{}:{:?}", alloy_primitives::hex::encode(self.slot), self.stype, self.rshift, self.last_and)
    }
}

type SlotHashMap = HashMap<Slot, Vec<Rc<RefCell<StorageElement>>>>;

struct Storage {
    loaded: SlotHashMap,
}
impl Storage {
    fn new() -> Self {
        Self{
            loaded: HashMap::new(),
        }
    }

    fn remove(&mut self, val: &Rc<RefCell<StorageElement>>) {
        self.loaded.get_mut(&val.borrow().slot).unwrap().retain(|x| x != val);
    }

    fn sstore(&mut self, slot: Element<Label>, rshift: u8, vtype: DynSolType) {
        let x = self.get(slot, true);
        x.borrow_mut().stype.set_type(vtype);
        x.borrow_mut().rshift = rshift;
    }

    fn sload(&mut self, slot: Element<Label>) -> Rc<RefCell<StorageElement>> {
        self.get(slot, false)
    }

    fn get(&mut self, slot: Element<Label>, is_write: bool) -> Rc<RefCell<StorageElement>> {
        let mut sl = slot.data;

        let mut rt = StorageType::Base(DynSolType::Uint(256));
        let mut x = slot.label;

        loop {
            match x {
                Some(Label::Keccak(_, vals)) if vals.len() == 2 => {
                    let key = if let Some(Label::Typed(ref v)) = vals[0].label {
                        v.clone()
                    } else {
                        DynSolType::Uint(256)
                    };
                    rt = StorageType::Map(key, Box::new(rt));
                    x = vals[1].label.clone();
                    sl = vals[1].data;
                },
                Some(Label::Keccak(_, vals)) if vals.len() == 1 => {
                    sl = vals[0].data;

                    x = vals[0].label.clone();

                    //FIXME: it's always u256??
                    if let StorageType::Base(b) = rt {
                        rt = StorageType::Base(DynSolType::Array(Box::new(b)));
                    } else {
                        rt = StorageType::Base(DynSolType::Array(Box::new(DynSolType::Uint(256))));
                    }
                },
                _ => {
                    break
                }
            }
        }

        let v = Rc::new(RefCell::new(StorageElement{
            slot: sl,
            stype: rt,
            rshift: 0,
            is_write,
            last_and: None,
            last_or2: None
        }));
        self.loaded.entry(sl).or_default().push(v.clone());
        v
    }
}

fn analyze(
    vm: &mut Vm<Label, CallDataImpl<Label>>,
    st: &mut Storage,
    ret: StepResult<Label>,
) -> Result<Option<usize>, Box<dyn std::error::Error>> {
    match ret {
        StepResult{op: op::PUSH0..=op::PUSH32, ..} =>
        {
            vm.stack.peek_mut()?.label = Some(Label::Constant);
        }

        StepResult{
            op: op::ADD |op::MUL |op::SUB |op::DIV |op::SDIV |op::MOD |op::SMOD |op::EXP |op::SIGNEXTEND |op::LT |op::GT |op::SLT |op::SGT |op::EQ |op::AND |op::OR |op::XOR |op::BYTE |op::SHL |op::SHR |op::SAR,
            fa: Some(Element{label: Some(Label::Constant), ..}),
            sa: Some(Element{label: Some(Label::Constant), ..}),
            ..
        } =>
        {
            vm.stack.peek_mut()?.label = Some(Label::Constant);
        }


        StepResult{
            op: op::ADD | op::MUL | op::SUB | op::XOR,
            fa: Some(Element{label: lb @ Some(Label::Sloaded(_) | Label::Typed(_)), ..}),
            sa: Some(Element{label: Some(Label::Constant), ..}),
            ..
        }
        | StepResult{
            op: op::ADD | op::MUL | op::SUB | op::XOR,
            sa: Some(Element{label: lb @ Some(Label::Sloaded(_) | Label::Typed(_)), ..}),
            fa: Some(Element{label: Some(Label::Constant), ..}),
            ..
        } =>
        {
            vm.stack.peek_mut()?.label = lb;
        }

        StepResult{op: op::NOT | op::ISZERO,
            fa: Some(Element{label: Some(Label::Constant), ..}),
            ..
        } =>
        {
            vm.stack.peek_mut()?.label = Some(Label::Constant);
        }

        StepResult{op: op::CALLVALUE, ..} =>
        {
            vm.stack.peek_mut()?.label = Some(Label::Typed(DynSolType::Uint(256)));
        }

        //TODO signextend & byte
        StepResult{op: op::ISZERO, fa: Some(Element{label: label @ Some(Label::Typed(DynSolType::Bool)), ..}), ..} =>
        {
            vm.stack.peek_mut()?.label = label;
        },

        // NOT WORKING on tests - dig in
        StepResult{op: op::SIGNEXTEND, fa: Some(Element{label: label @ Some(Label::Typed(_)), ..}), ..} =>
        {
            vm.stack.peek_mut()?.label = label;
        },


        StepResult{op: op::ADD, fa: Some(Element{label: label @ Some(Label::Keccak(_,_)), ..}), ..}
      | StepResult{op: op::ADD, sa: Some(Element{label: label @ Some(Label::Keccak(_,_)), ..}), ..} =>
        {
            vm.stack.peek_mut()?.label = label;
        },

        StepResult{op: op::SLOAD, fa: Some(slot), ..} =>
        {
            *vm.stack.peek_mut()? = Element{
                label: Some(Label::Sloaded(st.sload(slot))),
                data: VAL_1_B,
            };
        }

        StepResult{op: op::JUMPI, fa: Some(fa), ..} => {
            let other_pc = usize::try_from(fa)
                .expect("set to usize in vm.rs");
            return Ok(Some(other_pc));
        }

        StepResult{op: op::CALLER | op::ORIGIN | op::ADDRESS, ..} =>
        {
            *vm.stack.peek_mut()? = Element{
                label: Some(Label::Typed(DynSolType::Address)),
                data: VAL_1_B,
            };
        }

        StepResult{op: op::ISZERO, fa: Some(Element{label: Some(Label::Sloaded(sl)), ..}), ..} =>
        {
            vm.stack.peek_mut()?.label = Some(Label::IsZero(sl));
        }

        StepResult{op: op::ISZERO, fa: Some(Element{label: Some(Label::IsZero(sl)), ..}), ..} =>
        {
            sl.borrow_mut().stype.set_type(DynSolType::Bool);
        }

        StepResult{op: op::SIGNEXTEND, sa: Some(Element{label: Some(Label::Sloaded(sl)), ..}), fa: Some(s0), ..} =>
        {
            if s0.data < VAL_32_B {
                let s0: u8 = s0.data[31];
                sl.borrow_mut().stype.set_type(DynSolType::Int((s0 as usize + 1) * 8));
            }
        }

        StepResult{op: op::BYTE, sa: Some(Element{label: Some(Label::Sloaded(sl)), ..}), ..} =>
        {
            sl.borrow_mut().stype.set_type(DynSolType::FixedBytes(32))
        }

        StepResult{op: op::EQ,
            fa: Some(Element{label: Some(Label::Typed(tp)), ..}),
            sa: Some(Element{label: Some(Label::Sloaded(sl)), ..}), ..
        }
        | StepResult{op: op::EQ,
            sa: Some(Element{label: Some(Label::Typed(tp)), ..}),
            fa: Some(Element{label: Some(Label::Sloaded(sl)), ..}), ..
        } =>
        {
            sl.borrow_mut().stype.set_type(tp);
        },

        StepResult{op: op::OR,
            fa: Some(Element{label: Some(Label::Sloaded(sl)), ..}),
            sa: Some(tt @ Element{label: Some(Label::Typed(_)), ..}),
            ..
        }
        | StepResult{op: op::OR,
            fa: Some(tt @ Element{label: Some(Label::Typed(_)), ..}),
            sa: Some(Element{label: Some(Label::Sloaded(sl)), ..}),
            ..
        } =>
        {
            sl.borrow_mut().last_or2 = Some(tt);
            vm.stack.peek_mut()?.label = Some(Label::Sloaded(sl));
        }

        StepResult{op: op::OR,
            fa: Some(Element{label: Some(Label::Sloaded(sl)), ..}),
            sa: Some(tt @ Element{label: Some(Label::Constant), ..}),
            ..
        }
        | StepResult{op: op::OR,
            fa: Some(tt @ Element{label: Some(Label::Constant), ..}),
            sa: Some(Element{label: Some(Label::Sloaded(sl)), ..}),
            ..
        } =>
        {
            sl.borrow_mut().last_or2 = Some(tt);
            vm.stack.peek_mut()?.label = Some(Label::Sloaded(sl));
        }

        StepResult{op: op::AND, fa: Some(Element{label: label @ Some(Label::Typed(_)), ..}), ..}
      | StepResult{op: op::AND, sa: Some(Element{label: label @ Some(Label::Typed(_)), ..}), ..} =>
        {
            vm.stack.peek_mut()?.label = label;
        },

        StepResult{op: op::AND,
            fa: Some(Element{label: Some(Label::Sloaded(sl)), ..}),
            sa: Some(ot @ Element{label: Some(Label::Constant), ..}),
            ..
        }
        | StepResult{op: op::AND,
            sa: Some(Element{label: Some(Label::Sloaded(sl)), ..}),
            fa: Some(ot @ Element{label: Some(Label::Constant), ..}),
            ..
        } =>
        {
            let mask: U256 = ot.into();
            sl.borrow_mut().last_and = Some(mask);

            if let Some(t) = and_mask_to_type(mask) {
                sl.borrow_mut().stype.set_type(t);
            } else if mask == VAL_1 {
                // string, check for SSO
                sl.borrow_mut().stype.set_type(DynSolType::String);
            }
            vm.stack.peek_mut()?.label = Some(Label::Sloaded(sl));
        },

        StepResult{op: op::SSTORE, fa: Some(slot), sa: Some(value), ..} =>
        {
            if let Some(Label::Sloaded(ref sl)) = value.label {
                st.remove(sl);
            }

            match value.label {
                Some(Label::Typed(t)) => st.sstore(slot, 0, t),
                Some(Label::Sloaded(sl)) => {
                    let sbr = sl.borrow();
                    if let Some(lor) = &sbr.last_or2 {
                        if let Some(land) = sbr.last_and {
                            let tv = land.trailing_ones();

                            let qwe = land >> tv;
                            let sz = qwe.trailing_zeros();

                            let dt = match &lor.label {
                                Some(Label::Typed(tp)) => tp.clone(),
                                Some(Label::Sloaded(sl2)) => sl2.borrow().stype.get_internal_type(),
                                _ => if sz == 160 {
                                    DynSolType::Address
                                } else {
                                    DynSolType::Uint(sz)
                                }
                            };
                            st.sstore(slot, (tv / 8) as u8, dt);
                        } else {
                            st.sstore(slot, 0, sbr.stype.get_internal_type());
                        }
                    } else {
                        // println!("SET {:?} TO {:?} | {:?}", slot, sbr.stype.get_internal_type(), sbr);
                        st.sstore(slot, 0, sbr.stype.get_internal_type());
                    }
                },
                _ => st.sstore(slot, 0, DynSolType::Uint(256)),
            }
        }

        StepResult{op: op::DIV, fa: Some(Element{label: Some(Label::Sloaded(sl)), ..}), sa: Some(ot), ..} =>
        {
            let mask: U256 = ot.into();

            if mask > VAL_1 && (mask & (mask - VAL_1)).is_zero() && (mask.bit_len() - 1) % 8 == 0 {
                let nl = st.sload(Element{data: sl.borrow().slot, label: None});
                let bl = mask.bit_len() - 1;
                nl.borrow_mut().rshift = (bl / 8) as u8;
                vm.stack.peek_mut()?.label = Some(Label::Sloaded(nl));

                // TODO: postprocess this
                // sl.borrow_mut().stype.set_type(if bl == 160 { DynSolType::Address } else { DynSolType::Uint(bl) });
            } else {
                vm.stack.peek_mut()?.label = Some(Label::Sloaded(sl));
            }
        }

        StepResult{op: op::KECCAK256, fa: Some(fa), sa: Some(sa), ..} =>
        {
            let off = u32::try_from(fa)?;
            let sz = u32::try_from(sa)?;

            vm.stack.peek_mut()?.label = Some(Label::Keccak(0, vec![]));
            if sz == 64 {
                let (val, used) = vm.memory.load(off); // value
                let (sval, sused) = vm.memory.load(off + 32); // slot

                let mut depth = 0;
                let mut first = Element{data: val.data, label: None};
                let mut second = Element{data: sval.data, label: None};
                if used.len() == 1 {
                    let lb = used.first().unwrap().clone();
                    if let Label::Keccak(d, _) = lb {
                        depth = d + 1;
                    }
                    first.label = Some(lb);
                }
                if sused.len() == 1 {
                    let lb = sused.first().unwrap().clone();
                    if let Label::Keccak(d, _) = lb {
                        if d+1 > depth {
                            depth = d + 1;
                        }
                    }
                    second.label = Some(lb);
                }
                if depth < 6 {
                    vm.stack.peek_mut()?.label = Some(Label::Keccak(depth, vec![first, second]));
                }
            } else if sz == 32 {
                let mut depth = 0;
                let (mut val, _used) = vm.memory.load(off); // value
                if _used.len() == 1 {
                    let lb = _used.first().unwrap().clone();
                    if let Label::Keccak(d, _) = lb {
                        depth = d + 1;
                    }
                    if depth < 6 {
                        val.label = Some(lb);
                    }
                }

                let ustry = usize::try_from(&val);
                vm.stack.peek_mut()?.label = Some(Label::Keccak(depth, vec![val]));
                if let Ok(v) = ustry {
                    if v < KEC_PRECALC.len() {
                        vm.stack.peek_mut()?.data = KEC_PRECALC[v];
                    }
                }
            }
        },
        _ => (),
    };
    Ok(None)
}


fn analyze_rec(
    mut vm: Vm<Label, CallDataImpl<Label>>,
    st: &mut Storage,
    gas_limit: u32,
    depth: u32,
) -> u32 {
    let mut gas_used = 0;

    while !vm.stopped {
        if cfg!(feature = "trace_storage") {
            println!("{:?}\n", vm);
            println!("storage: {:?}\n", st.loaded);
        }
        let ret = match vm.step() {
            Ok(v) => v,
            Err(_e) => {
                // println!("{}", _e);
                break;
            }
        };
        gas_used += ret.gas_used;
        if gas_used > gas_limit {
            break;
        }


        match analyze(&mut vm, st, ret) {
            Err(_) => {
                // println!("errbrk");
                break;
            },
            Ok(Some(other_pc)) => {
                if depth < 8 && other_pc < vm.code.len()  {
                    let mut cloned = vm.clone();
                    cloned.pc = other_pc;
                    gas_used += analyze_rec(cloned, st, (gas_limit - gas_used) / 2, depth + 1);
                }
            },
            Ok(None) => {},
        }
    }

    gas_used
}

fn analyze_one_function(code: &[u8], selector: Selector, arguments: &[DynSolType], is_fallback: bool, gas_limit: u32)  -> SlotHashMap {
    if cfg!(feature = "trace_storage") {
        println!("analyze selector {}\n", alloy_primitives::hex::encode(selector));
    }

    let calldata = CallDataImpl::<Label>::new(selector, arguments);
    let mut vm = Vm::new(
        code,
        &calldata
    );

    let mut st = Storage::new();
    let mut gas_used = 0;

    if !is_fallback {
        if let Some(g) = execute_until_function_start(&mut vm, gas_limit) {
            gas_used += g;
        } else {
            return st.loaded;
        }
    }

    #[allow(unused_assignments)]
    if gas_used < gas_limit {
        gas_used += analyze_rec(vm, &mut st, gas_limit - gas_used, 0);
    }

    st.loaded.into_iter().map(|(k, v)|
        (k, {
            let x = v.iter().find(|e| e.borrow().stype == StorageType::Base(DynSolType::String));
            if let Some(val) = x {
                vec![val.clone()]
            } else {

                let qwe: Vec<_> = v.clone().into_iter().filter(|e| {
                    let br = e.borrow();
                    if let StorageType::Map(_, _) = br.stype {
                        br.rshift == 0
                    } else {
                        false
                    }
                }).collect();
                if !qwe.is_empty() {
                    //TODO: return other rshift as struct elems
                    qwe
                } else {
                    v
                }

            }
        })
    ).collect()
}

pub fn contract_storage<I, D>(code: &[u8], functions: I, gas_limit: u32) -> Vec<StorageRecord>
    where I: IntoIterator<Item = (Selector, usize, D)>,
            D: AsRef<[DynSolType]>
{
    let real_gas_limit = if gas_limit == 0 {
        1e6 as u32
    } else {
        gas_limit
    };

    let mut xr: BTreeMap<(Slot, u8), Vec<(Selector, StorageElement)> > = BTreeMap::new();

    for (sel, _pc, arguments) in functions.into_iter() {
        let st = analyze_one_function(code, sel, arguments.as_ref(), false, real_gas_limit);
        for (slot, loaded) in st.into_iter() {
            for ld in loaded.into_iter() {
                // println!(
                //     "{} | {} | {:?}",
                //     alloy_primitives::hex::encode(slot),
                //     alloy_primitives::hex::encode(sel),
                //     ld
                // );
                let v = (*ld).borrow();
                xr.entry((slot, v.rshift)).or_default().push((sel, v.clone()));
            }
        }
    }

    // fallback()
    const FALLBACK_SELECTOR: Selector = [0xff, 0xff, 0xff, 0xff];
    let st = analyze_one_function(code, FALLBACK_SELECTOR, &[], true, real_gas_limit);
    for (slot, loaded) in st.into_iter() {
        for ld in loaded.into_iter() {
            let v = (*ld).borrow();
            let qq = v.clone();
            xr.entry((slot, v.rshift)).or_default().push((FALLBACK_SELECTOR, qq));
        }
    }

    let mut ret: Vec<StorageRecord> = Vec::with_capacity(xr.len());

    for ((slot, offset), hmap) in xr.into_iter() {
        let mut reads: BTreeSet<Selector> = BTreeSet::new();
        let mut writes: BTreeSet<Selector> = BTreeSet::new();

        let mut best_type: StorageType = StorageType::Base(DynSolType::Uint(256));
        let mut best_score = best_type.get_score();

        for (selector, selem) in hmap.into_iter() {
            if selector != FALLBACK_SELECTOR {
                if selem.is_write {
                    writes.insert(selector);
                } else {
                    reads.insert(selector);
                }
            }

            let tt = selem.stype;

            let score = tt.get_score();
            if score > best_score {
                // println!(
                //     "{:?} => {:?} ({} => {})",
                //     best_type,
                //     tt,
                //     best_score,
                //     score
                // );
                // best_type = selem.stype;
                best_type = tt;
                best_score = score;
            }
        }

        ret.push(StorageRecord{
            slot,
            offset,
            r#type: format!("{:?}", best_type),
            reads: reads.into_iter().collect(),
            writes: writes.into_iter().collect(),
        })
    }

    if cfg!(feature = "trace_storage") {
        for r in ret.iter() {
            println!("slot {} off {}", alloy_primitives::hex::encode(r.slot), r.offset);
            println!(" type: {}", r.r#type);
            println!(" reads: {:?}", r.reads.iter().map(alloy_primitives::hex::encode).collect::<Vec<_>>());
            println!(" writes: {:?}", r.writes.iter().map(alloy_primitives::hex::encode).collect::<Vec<_>>());
        }
    }
    ret
}
