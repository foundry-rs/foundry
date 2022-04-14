use crate::trace::{CallTrace, LogCallOrder, RawOrDecodedLog};

/// A node in the arena
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct CallTraceNode {
    /// Parent node index in the arena
    pub parent: Option<usize>,
    /// Children node indexes in the arena
    pub children: Vec<usize>,
    /// This node's index in the arena
    pub idx: usize,
    /// The call trace
    pub trace: CallTrace,
    /// Logs
    #[serde(skip)]
    pub logs: Vec<RawOrDecodedLog>,
    /// Ordering of child calls and logs
    pub ordering: Vec<LogCallOrder>,
}
