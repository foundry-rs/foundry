use ethers::abi::FunctionExt;
use std::collections::BTreeMap;
use ethers::{
	abi::{Abi, RawLog},
	types::{Address, H160},
};

/// Call trace of a tx
#[derive(Clone, Default, Debug)]
pub struct CallTrace {
	pub depth: usize,
	pub location: usize,
    /// Successful
    pub success: bool,
    /// Callee
    pub addr: H160,
    /// Creation
    pub created: bool,
    /// Call data, including function selector (if applicable)
    pub data: Vec<u8>,
    /// Gas cost
    pub cost: u64,
    /// Output
    pub output: Vec<u8>,
    /// Logs
    pub logs: Vec<RawLog>,
    /// inner calls
    pub inner: Vec<CallTrace>,
}

impl CallTrace {
	pub fn add_trace(&mut self, new_trace: Self) {
		if new_trace.depth == 0 {
			// overwrite
			self.update(new_trace);
		} else if self.depth == new_trace.depth - 1 {
			self.inner.push(new_trace);
		} else {
			self.inner.last_mut().expect("Disconnected trace").add_trace(new_trace);
		}
	}

	fn update(&mut self, new_trace: Self) {
		self.success = new_trace.success;
		self.addr = new_trace.addr;
		self.cost = new_trace.cost;
		self.output = new_trace.output;
		self.logs = new_trace.logs;
		self.data = new_trace.data;
		self.addr = new_trace.addr;
		// we dont update inner because the temporary new_trace doesnt track inner calls
	}

	pub fn update_trace(&mut self, new_trace: Self) {
		if new_trace.depth == 0 {
			// overwrite
			self.update(new_trace);
		} else if self.depth == new_trace.depth - 1 {
			self.inner[new_trace.location].update(new_trace);
		} else {
			self.inner.last_mut().expect("Disconnected trace update").update_trace(new_trace);
		}
	}

	pub fn location(&self, new_trace: &Self) -> usize {
		if new_trace.depth == 0 {
			0
		} else if self.depth == new_trace.depth - 1 {
			self.inner.len()
		} else {
			self.inner.last().expect("Disconnected trace location").location(new_trace)
		}
	}

	pub fn pretty_print(&self, contracts: &BTreeMap<String, (Abi, Address, Vec<String>)>) {
		if let Some((name, (abi, _addr, _other))) = contracts.iter().find(|(_key, (_abi, addr, _other))| {
			addr == &self.addr
		}) {
			for (func_name, overloaded_funcs) in abi.functions.iter() {
				for func in overloaded_funcs.iter() {
					if func.selector() == self.data[0..4] {
						println!("{}.{}({:?})", name, func_name, func.decode_input(&self.data[4..]).unwrap());
					}
				}
			}
			self.inner.iter().for_each(|inner| inner.pretty_print(contracts));
		}
		
	}
}