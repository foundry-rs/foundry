use tokio::sync::{mpsc, oneshot};

use crate::{APDUAnswer, APDUCommand, LedgerError};

mod error;
pub use error::NativeTransportError;

pub mod hid;
use hid::TransportNativeHID;

/// A packet exchange request.
struct APDUExchange {
    /// The command to send to the device.
    command: APDUCommand,
    /// The channel to send the answer back on.
    answer: oneshot::Sender<Result<APDUAnswer, LedgerError>>,
}

impl APDUExchange {
    /// Create a new exchange request.
    fn new(command: APDUCommand) -> (Self, oneshot::Receiver<Result<APDUAnswer, LedgerError>>) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                command,
                answer: tx,
            },
            rx,
        )
    }
}

/// A task that manages Ledger packet exchange.
struct LedgerTask {
    ledger: TransportNativeHID,
    rx: tokio::sync::mpsc::Receiver<APDUExchange>,
}

impl LedgerTask {
    /// Create a new task.
    fn new(ledger: TransportNativeHID) -> (Self, tokio::sync::mpsc::Sender<APDUExchange>) {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        (Self { ledger, rx }, tx)
    }

    /// Spawn the task that will run Ledger protocols.
    fn spawn(mut self) {
        let fut = async move {
            while let Some(exchange) = self.rx.recv().await {
                // blocking IO
                let answer = self.ledger.exchange(&exchange.command);
                let answer = exchange.answer.send(answer);
                if let Err(Err(err)) = answer {
                    tracing::error!(err = %err, "could not send answer to exchange");
                }
            }
        };

        std::thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(fut);
        });
    }
}

/// A handle to the Ledger device. This handle is not clone, as it is critical
/// that only one connection to the device is active at a time. APDUs may NOT
/// be interleaved.
#[derive(Debug)]
pub struct LedgerHandle {
    tx: mpsc::Sender<APDUExchange>,
}

impl LedgerHandle {
    /// Init a handle, and spawn a task to manage Ledger packet exchange.
    pub fn init() -> Result<Self, LedgerError> {
        let ledger = TransportNativeHID::new()?;
        let (task, tx) = LedgerTask::new(ledger);
        task.spawn();
        Ok(Self { tx })
    }

    /// Exchange a packet with the device.
    pub async fn exchange(&self, apdu: APDUCommand) -> Result<APDUAnswer, LedgerError> {
        let (exchange, rx) = APDUExchange::new(apdu);
        self.tx
            .send(exchange)
            .await
            .map_err(|_| LedgerError::BackendGone)?;
        rx.await.map_err(|_| LedgerError::BackendGone)?
    }
}
