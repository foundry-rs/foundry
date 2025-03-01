/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::client::retries::classifiers::run_classifiers_on_ctx;
use aws_smithy_runtime_api::box_error::BoxError;
use aws_smithy_runtime_api::client::connection::ConnectionMetadata;
use aws_smithy_runtime_api::client::interceptors::context::{
    AfterDeserializationInterceptorContextRef, BeforeTransmitInterceptorContextMut,
};
use aws_smithy_runtime_api::client::interceptors::Intercept;
use aws_smithy_runtime_api::client::retries::classifiers::RetryAction;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_types::config_bag::{ConfigBag, Storable, StoreReplace};
use aws_smithy_types::retry::{ReconnectMode, RetryConfig};
use std::fmt;
use std::sync::{Arc, Mutex};
use tracing::{debug, error};

/// An interceptor for poisoning connections in response to certain events.
///
/// This interceptor, when paired with a compatible connection, allows the connection to be
/// poisoned in reaction to certain events *(like receiving a transient error.)* This allows users
/// to avoid sending requests to a server that isn't responding. This can increase the load on a
/// server, because more connections will be made overall.
///
/// **In order for this interceptor to work,** the configured connection must interact with the
/// "connection retriever" stored in an HTTP request's `extensions` map. For an example of this,
/// see [`HyperConnector`]. When a connection is made available to the retriever, this interceptor
/// will call a `.poison` method on it, signalling that the connection should be dropped. It is
/// up to the connection implementer to handle this.
///
/// [`HyperConnector`]: https://github.com/smithy-lang/smithy-rs/blob/26a914ece072bba2dd9b5b49003204b70e7666ac/rust-runtime/aws-smithy-runtime/src/client/http/hyper_014.rs#L347
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct ConnectionPoisoningInterceptor {}

impl ConnectionPoisoningInterceptor {
    /// Create a new `ConnectionPoisoningInterceptor`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Intercept for ConnectionPoisoningInterceptor {
    fn name(&self) -> &'static str {
        "ConnectionPoisoningInterceptor"
    }

    fn modify_before_transmit(
        &self,
        context: &mut BeforeTransmitInterceptorContextMut<'_>,
        _runtime_components: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let capture_smithy_connection = CaptureSmithyConnection::new();
        context
            .request_mut()
            .add_extension(capture_smithy_connection.clone());
        cfg.interceptor_state().store_put(capture_smithy_connection);

        Ok(())
    }

    fn read_after_deserialization(
        &self,
        context: &AfterDeserializationInterceptorContextRef<'_>,
        runtime_components: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let reconnect_mode = cfg
            .load::<RetryConfig>()
            .map(RetryConfig::reconnect_mode)
            .unwrap_or(ReconnectMode::ReconnectOnTransientError);
        let captured_connection = cfg.load::<CaptureSmithyConnection>().cloned();
        let retry_classifier_result =
            run_classifiers_on_ctx(runtime_components.retry_classifiers(), context.inner());
        let error_is_transient = retry_classifier_result == RetryAction::transient_error();
        let connection_poisoning_is_enabled =
            reconnect_mode == ReconnectMode::ReconnectOnTransientError;

        if error_is_transient && connection_poisoning_is_enabled {
            debug!("received a transient error, marking the connection for closure...");

            if let Some(captured_connection) = captured_connection.and_then(|conn| conn.get()) {
                captured_connection.poison();
                debug!("the connection was marked for closure")
            } else {
                error!(
                    "unable to mark the connection for closure because no connection was found! The underlying HTTP connector never set a connection."
                );
            }
        }

        Ok(())
    }
}

type LoaderFn = dyn Fn() -> Option<ConnectionMetadata> + Send + Sync;

/// State for a middleware that will monitor and manage connections.
#[derive(Clone, Default)]
pub struct CaptureSmithyConnection {
    loader: Arc<Mutex<Option<Box<LoaderFn>>>>,
}

impl CaptureSmithyConnection {
    /// Create a new connection monitor.
    pub fn new() -> Self {
        Self {
            loader: Default::default(),
        }
    }

    /// Set the retriever that will capture the `hyper` connection.
    pub fn set_connection_retriever<F>(&self, f: F)
    where
        F: Fn() -> Option<ConnectionMetadata> + Send + Sync + 'static,
    {
        *self.loader.lock().unwrap() = Some(Box::new(f));
    }

    /// Get the associated connection metadata.
    pub fn get(&self) -> Option<ConnectionMetadata> {
        match self.loader.lock().unwrap().as_ref() {
            Some(loader) => loader(),
            None => {
                tracing::debug!("no loader was set on the CaptureSmithyConnection");
                None
            }
        }
    }
}

impl fmt::Debug for CaptureSmithyConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CaptureSmithyConnection")
    }
}

impl Storable for CaptureSmithyConnection {
    type Storer = StoreReplace<Self>;
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    #[allow(clippy::redundant_clone)]
    fn retrieve_connection_metadata() {
        let retriever = CaptureSmithyConnection::new();
        let retriever_clone = retriever.clone();
        assert!(retriever.get().is_none());
        retriever.set_connection_retriever(|| {
            Some(
                ConnectionMetadata::builder()
                    .proxied(true)
                    .poison_fn(|| {})
                    .build(),
            )
        });

        assert!(retriever.get().is_some());
        assert!(retriever_clone.get().is_some());
    }
}
