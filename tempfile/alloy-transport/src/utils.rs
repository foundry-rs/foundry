use crate::{TransportError, TransportResult};
use serde::Serialize;
use serde_json::value::{to_raw_value, RawValue};
use std::future::Future;
use url::Url;

/// Convert to a `Box<RawValue>` from a `Serialize` type, mapping the error
/// to a `TransportError`.
pub fn to_json_raw_value<S>(s: &S) -> TransportResult<Box<RawValue>>
where
    S: Serialize,
{
    to_raw_value(s).map_err(TransportError::ser_err)
}

/// Guess whether the URL is local, based on the hostname.
///
/// The output of this function is best-efforts, and should be checked if
/// possible. It simply returns `true` if the connection has no hostname,
/// or the hostname is `localhost` or `127.0.0.1`.
pub fn guess_local_url(s: impl AsRef<str>) -> bool {
    fn _guess_local_url(url: &str) -> bool {
        url.parse::<Url>().is_ok_and(|url| {
            url.host_str().map_or(true, |host| host == "localhost" || host == "127.0.0.1")
        })
    }
    _guess_local_url(s.as_ref())
}

#[doc(hidden)]
pub trait Spawnable {
    /// Spawn the future as a task.
    ///
    /// In WASM this will be a `wasm-bindgen-futures::spawn_local` call, while
    /// in native it will be a `tokio::spawn` call.
    fn spawn_task(self);
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> Spawnable for T
where
    T: Future<Output = ()> + Send + 'static,
{
    fn spawn_task(self) {
        tokio::spawn(self);
    }
}

#[cfg(target_arch = "wasm32")]
impl<T> Spawnable for T
where
    T: Future<Output = ()> + 'static,
{
    fn spawn_task(self) {
        #[cfg(not(feature = "wasm-bindgen"))]
        panic!("The 'wasm-bindgen' feature must be enabled");

        #[cfg(feature = "wasm-bindgen")]
        wasm_bindgen_futures::spawn_local(self);
    }
}
