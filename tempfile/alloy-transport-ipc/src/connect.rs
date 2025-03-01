use interprocess::local_socket as ls;
use std::io;

pub(crate) fn to_name(path: &std::ffi::OsStr) -> io::Result<ls::Name<'_>> {
    if cfg!(windows) && !path.as_encoded_bytes().starts_with(br"\\.\pipe\") {
        ls::ToNsName::to_ns_name::<ls::GenericNamespaced>(path)
    } else {
        ls::ToFsName::to_fs_name::<ls::GenericFilePath>(path)
    }
}

/// An IPC Connection object.
#[derive(Clone, Debug)]
pub struct IpcConnect<T> {
    inner: T,
}

impl<T> IpcConnect<T> {
    /// Create a new IPC connection object for any type T that can be converted into
    /// `IpcConnect<T>`.
    pub const fn new(inner: T) -> Self
    where
        Self: alloy_pubsub::PubSubConnect,
    {
        Self { inner }
    }
}

macro_rules! impl_connect {
    ($target:ty => | $inner:ident | $map:expr) => {
        impl From<$target> for IpcConnect<$target> {
            fn from(inner: $target) -> Self {
                Self { inner }
            }
        }

        impl From<IpcConnect<$target>> for $target {
            fn from(this: IpcConnect<$target>) -> $target {
                this.inner
            }
        }

        impl alloy_pubsub::PubSubConnect for IpcConnect<$target> {
            fn is_local(&self) -> bool {
                true
            }

            async fn connect(
                &self,
            ) -> Result<alloy_pubsub::ConnectionHandle, alloy_transport::TransportError> {
                let $inner = &self.inner;
                let inner = $map;
                let name = to_name(inner).map_err(alloy_transport::TransportErrorKind::custom)?;
                crate::IpcBackend::connect(name)
                    .await
                    .map_err(alloy_transport::TransportErrorKind::custom)
            }
        }
    };
}

impl_connect!(std::ffi::OsString => |s| s.as_os_str());
impl_connect!(std::path::PathBuf => |s| s.as_os_str());
impl_connect!(String => |s| s.as_ref());

#[cfg(unix)]
impl_connect!(std::ffi::CString => |s| {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::OsStr::from_bytes(s.to_bytes())
});
