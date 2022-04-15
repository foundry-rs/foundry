//! Support for "cheat codes" / bypass functions

use parking_lot::RwLock;
use std::sync::Arc;

/// Manages user modifications that may affect the node's behavior
///
/// Contains the state of executed, non-eth standard cheat code RPC
#[derive(Debug, Clone, Default)]
pub struct CheatsManager {
    /// shareable state
    state: Arc<RwLock<CheatsState>>,
}

/// Container type for all the state variables
#[derive(Debug, Clone, Default)]
pub struct CheatsState {}
