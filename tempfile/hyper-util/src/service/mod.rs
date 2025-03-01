//! Service utilities.

#[cfg(feature = "service")]
mod glue;
#[cfg(any(feature = "client-legacy", feature = "service"))]
mod oneshot;

#[cfg(feature = "service")]
pub use self::glue::{TowerToHyperService, TowerToHyperServiceFuture};
#[cfg(any(feature = "client-legacy", feature = "service"))]
pub(crate) use self::oneshot::Oneshot;
