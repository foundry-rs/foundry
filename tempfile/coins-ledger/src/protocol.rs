use crate::{Ledger, LedgerError};

/// A Ledger Recovery. A the device to perform
/// some operation. Protocols are run in a task, and may send multiple
/// commands to the device, and receive multiple responses. The protocol has
/// exclusive access to the transport while it is running.
///
/// The protocol may fail, and the [`LedgerProtocol::recover`] function will
/// be invoked. The protocol may also fail to recover, in which case future
/// uses of the device may fail.
pub trait LedgerProtocol {
    /// The output of the protocol.
    type Output;

    /// Run the protocol. This sends commands to the device, and receives
    /// responses. The transport is locked while this function is running.
    /// If the protocol fails, the app may be in an undefined state, and
    /// the [`LedgerProtocol::recover`] function will be invoked.
    ///
    fn execute(&mut self, transport: &mut Ledger) -> Result<Self::Output, LedgerError>;

    /// Run recovery if the protocol fails.
    ///
    /// This is invoked after the protocol fails. This function should attempt
    /// to recover the app on the device to a known state.
    ///
    /// Multi-APDU protocols MUST override this function. The recommended
    /// implementation is to retrieve a pubkey from the device twice.
    fn recover(&self, _transport: &mut Ledger) -> Result<(), LedgerError> {
        Ok(())
    }

    /// Run the protocol. This sends commands to the device, and receives
    /// responses. The transport is locked while this function is running.
    fn run(&mut self, transport: &mut Ledger) -> Result<Self::Output, LedgerError> {
        match self.execute(transport) {
            Ok(output) => Ok(output),
            Err(e) => {
                // TODO: make less ugly
                #[cfg(target_arch = "wasm32")]
                log::error!("Protocol failed, running recovery: {}", e);
                #[cfg(not(target_arch = "wasm32"))]
                tracing::error!(err = %e, "Protocol failed, running recovery.");

                if let Err(e) = self.recover(transport) {
                    #[cfg(target_arch = "wasm32")]
                    log::error!("Recovery failed: {}", e);
                    #[cfg(not(target_arch = "wasm32"))]
                    tracing::error!(err = %e, "Recovery failed.");
                }

                Err(e)
            }
        }
    }
}
