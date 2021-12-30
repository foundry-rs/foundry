
use sputnik::{
    backend::Backend,
    executor::stack::{
        Log, PrecompileFailure, PrecompileOutput, PrecompileSet, StackExecutor, StackExitKind,
        StackState, StackSubstateMetadata,
    },
    gasometer, Capture, Config, Context, CreateScheme, ExitError, ExitReason, ExitRevert,
    ExitSucceed, Handler, Runtime, Transfer, Resolve, Machine
};
use std::rc::Rc;
/// EVM runtime.
///
/// The runtime wraps an EVM `Machine` with support of return data and context.
pub struct ForgeRuntime<'config> {
	pub inner: Runtime<'config>,
}

impl<'config> ForgeRuntime<'config> {
	pub fn new(
		code: Rc<Vec<u8>>,
		data: Rc<Vec<u8>>,
		context: Context,
		config: &'config Config,
	) -> Self {
		Self {
			inner: Runtime::new(code, data, context, config)
		}
	}
	/// Step the runtime.
	pub fn step<'a, H: Handler>(
		&'a mut self,
		handler: &mut H,
	) -> Result<(), Capture<ExitReason, Resolve<'a, 'config, H>>> {
		
		self.inner.step(handler)
	}

	/// Get a reference to the machine.
	pub fn machine(&self) -> &Machine {
		&self.inner.machine()
	}

	/// Loop stepping the runtime until it stops.
	pub fn run<'a, H: Handler>(
		&'a mut self,
		handler: &mut H,
	) -> Capture<ExitReason, ()> {
		let mut done = false;
		let mut res = Capture::Exit(ExitReason::Succeed(ExitSucceed::Returned));
		while !done {
			{
				println!("stack {:?}", self.inner.machine().stack());	
			}
			
			let r = self.inner.step(handler);
			match r {
				Ok(()) => {}
				Err(e) => { done = true;
					match e {
						Capture::Exit(s) => {res = Capture::Exit(s)},
			            Capture::Trap(_) => unreachable!("Trap is Infallible"),	
					}
				}
			}
		}
		res
	}
}

