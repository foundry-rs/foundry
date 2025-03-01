use thiserror::Error;

// Mock the type in other target_os
#[cfg(not(target_os = "linux"))]
mod nix {
    #[derive(thiserror::Error, Debug, Copy, Clone)]
    pub enum Error {
        #[error("")]
        Unimplemented,
    }
}

/// Ledger transport errors
#[derive(Error, Debug)]
pub enum NativeTransportError {
    /// Device not found error
    #[error("Ledger device not found")]
    DeviceNotFound,
    /// Device open error.
    #[error("Error opening device. {0}. Hint: This usually means that the device is already in use by another transport instance.")]
    CantOpen(hidapi_rusb::HidError),
    /// SequenceMismatch
    #[error("Sequence mismatch. Got {got} from device. Expected {expected}")]
    SequenceMismatch {
        /// The sequence returned by the device
        got: u16,
        /// The expected sequence
        expected: u16,
    },
    /// Communication error
    #[error("Ledger device: communication error `{0}`")]
    Comm(&'static str),
    /// Ioctl error
    #[error(transparent)]
    Ioctl(#[from] nix::Error),
    /// i/o error
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// HID error
    #[error(transparent)]
    Hid(#[from] hidapi_rusb::HidError),
    /// UT8F error
    #[error(transparent)]
    UTF8(#[from] std::str::Utf8Error),
    /// Termux USB FD env var does not exist or fails to parse. This error is
    /// only returned by android-specific code paths, and may be safely ignored
    /// by non-android users
    #[error("Invalid TERMUX_USB_FD variable. Are you using termux-usb?")]
    InvalidTermuxUsbFd,
}
