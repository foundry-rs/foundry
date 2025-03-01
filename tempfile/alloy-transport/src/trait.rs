use crate::{BoxTransport, IntoBoxTransport, TransportError, TransportFut};
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use tower::Service;

/// A `Transport` manages the JSON-RPC request/response lifecycle.
///
/// `Transports` should be instantiated via the [`TransportConnect`] trait.
///
/// Transports are responsible for the following:
///
/// - Communicating with the RPC server.
/// - Managing any ongoing connection or communication resource.
/// - Associating responses with requests.
/// - Associating notifications with subscriptions.
///
/// As a result, a `Transport` may be a simple HTTP client, or a collection of
/// long-lived tasks.
///
/// ## Implementing `Transport`
///
/// This trait is blanket implemented for all appropriate types. To implement
/// this trait, you must implement the [`tower::Service`] trait with the
/// appropriate associated types. It cannot be implemented directly.
///
/// ### ⚠️ Always implement `Clone` ⚠️
///
/// [`Clone`] is not a bound on `Transport`, however, transports generally may
/// not be used as expected unless they implement `Clone`. For example, only
/// cloneable transports may be used by the `RpcClient` in `alloy-rpc-client`
/// to send RPC requests, and [`BoxTransport`] may only be used to type-erase
/// cloneable transports.
///
/// If you are implementing a transport, make sure it is [`Clone`].
///
/// [`TransportConnect`]: crate::TransportConnect
pub trait Transport:
    Service<
        RequestPacket,
        Response = ResponsePacket,
        Error = TransportError,
        Future = TransportFut<'static>,
    > + Send
    + Sync
    + 'static
{
    /// Convert this transport into a boxed trait object.
    fn boxed(self) -> BoxTransport
    where
        Self: IntoBoxTransport,
    {
        BoxTransport::new(self)
    }

    /// Make a boxed trait object by cloning this transport.
    fn as_boxed(&self) -> BoxTransport
    where
        Self: IntoBoxTransport,
    {
        self.clone().boxed()
    }
}

impl<T> Transport for T where
    T: Service<
            RequestPacket,
            Response = ResponsePacket,
            Error = TransportError,
            Future = TransportFut<'static>,
        > + Send
        + Sync
        + 'static
{
}
