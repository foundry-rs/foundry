use crate::{BoxTransport, TransportError};
use futures_utils_wasm::impl_future;

/// Connection details for a transport.
///
/// This object captures the information necessary to establish a transport,
/// and may encapsulate reconnection logic.
///
/// ## Why implement `TransportConnect`?
///
/// Users may want to implement transport-connect for the following reasons:
/// - You want to customize a `reqwest::Client` before using it.
/// - You need to provide special authentication information to a remote provider.
/// - You have implemented a custom [`Transport`](crate::Transport).
/// - You require a specific websocket reconnection strategy.
pub trait TransportConnect: Sized + Send + Sync + 'static {
    /// Returns `true` if the transport connects to a local resource.
    fn is_local(&self) -> bool;

    /// Connect to the transport, returning a `Transport` instance.
    fn get_transport(&self) -> impl_future!(<Output = Result<BoxTransport, TransportError>>);
}
