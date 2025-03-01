/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Runtime plugins that provide defaults for clients.
//!
//! Note: these are the absolute base-level defaults. They may not be the defaults
//! for _your_ client, since many things can change these defaults on the way to
//! code generating and constructing a full client.

use crate::client::http::body::content_length_enforcement::EnforceContentLengthRuntimePlugin;
use crate::client::identity::IdentityCache;
use crate::client::retries::strategy::standard::TokenBucketProvider;
use crate::client::retries::strategy::StandardRetryStrategy;
use crate::client::retries::RetryPartition;
use aws_smithy_async::rt::sleep::default_async_sleep;
use aws_smithy_async::time::SystemTimeSource;
use aws_smithy_runtime_api::box_error::BoxError;
use aws_smithy_runtime_api::client::behavior_version::BehaviorVersion;
use aws_smithy_runtime_api::client::http::SharedHttpClient;
use aws_smithy_runtime_api::client::runtime_components::{
    RuntimeComponentsBuilder, SharedConfigValidator,
};
use aws_smithy_runtime_api::client::runtime_plugin::{
    Order, SharedRuntimePlugin, StaticRuntimePlugin,
};
use aws_smithy_runtime_api::client::stalled_stream_protection::StalledStreamProtectionConfig;
use aws_smithy_runtime_api::shared::IntoShared;
use aws_smithy_types::config_bag::{ConfigBag, FrozenLayer, Layer};
use aws_smithy_types::retry::RetryConfig;
use aws_smithy_types::timeout::TimeoutConfig;
use std::borrow::Cow;
use std::time::Duration;

fn default_plugin<CompFn>(name: &'static str, components_fn: CompFn) -> StaticRuntimePlugin
where
    CompFn: FnOnce(RuntimeComponentsBuilder) -> RuntimeComponentsBuilder,
{
    StaticRuntimePlugin::new()
        .with_order(Order::Defaults)
        .with_runtime_components((components_fn)(RuntimeComponentsBuilder::new(name)))
}

fn layer<LayerFn>(name: &'static str, layer_fn: LayerFn) -> FrozenLayer
where
    LayerFn: FnOnce(&mut Layer),
{
    let mut layer = Layer::new(name);
    (layer_fn)(&mut layer);
    layer.freeze()
}

/// Runtime plugin that provides a default connector.
pub fn default_http_client_plugin() -> Option<SharedRuntimePlugin> {
    let _default: Option<SharedHttpClient> = None;
    #[cfg(feature = "connector-hyper-0-14-x")]
    let _default = crate::client::http::hyper_014::default_client();

    _default.map(|default| {
        default_plugin("default_http_client_plugin", |components| {
            components.with_http_client(Some(default))
        })
        .into_shared()
    })
}

/// Runtime plugin that provides a default async sleep implementation.
pub fn default_sleep_impl_plugin() -> Option<SharedRuntimePlugin> {
    default_async_sleep().map(|default| {
        default_plugin("default_sleep_impl_plugin", |components| {
            components.with_sleep_impl(Some(default))
        })
        .into_shared()
    })
}

/// Runtime plugin that provides a default time source.
pub fn default_time_source_plugin() -> Option<SharedRuntimePlugin> {
    Some(
        default_plugin("default_time_source_plugin", |components| {
            components.with_time_source(Some(SystemTimeSource::new()))
        })
        .into_shared(),
    )
}

/// Runtime plugin that sets the default retry strategy, config (disabled), and partition.
pub fn default_retry_config_plugin(
    default_partition_name: impl Into<Cow<'static, str>>,
) -> Option<SharedRuntimePlugin> {
    let retry_partition = RetryPartition::new(default_partition_name);
    Some(
        default_plugin("default_retry_config_plugin", |components| {
            components
                .with_retry_strategy(Some(StandardRetryStrategy::new()))
                .with_config_validator(SharedConfigValidator::base_client_config_fn(
                    validate_retry_config,
                ))
                .with_interceptor(TokenBucketProvider::new(retry_partition.clone()))
        })
        .with_config(layer("default_retry_config", |layer| {
            layer.store_put(RetryConfig::disabled());
            layer.store_put(retry_partition);
        }))
        .into_shared(),
    )
}

fn validate_retry_config(
    components: &RuntimeComponentsBuilder,
    cfg: &ConfigBag,
) -> Result<(), BoxError> {
    if let Some(retry_config) = cfg.load::<RetryConfig>() {
        if retry_config.has_retry() && components.sleep_impl().is_none() {
            Err("An async sleep implementation is required for retry to work. Please provide a `sleep_impl` on \
                 the config, or disable timeouts.".into())
        } else {
            Ok(())
        }
    } else {
        Err(
            "The default retry config was removed, and no other config was put in its place."
                .into(),
        )
    }
}

/// Runtime plugin that sets the default timeout config (no timeouts).
pub fn default_timeout_config_plugin() -> Option<SharedRuntimePlugin> {
    Some(
        default_plugin("default_timeout_config_plugin", |components| {
            components.with_config_validator(SharedConfigValidator::base_client_config_fn(
                validate_timeout_config,
            ))
        })
        .with_config(layer("default_timeout_config", |layer| {
            layer.store_put(TimeoutConfig::disabled());
        }))
        .into_shared(),
    )
}

fn validate_timeout_config(
    components: &RuntimeComponentsBuilder,
    cfg: &ConfigBag,
) -> Result<(), BoxError> {
    if let Some(timeout_config) = cfg.load::<TimeoutConfig>() {
        if timeout_config.has_timeouts() && components.sleep_impl().is_none() {
            Err("An async sleep implementation is required for timeouts to work. Please provide a `sleep_impl` on \
                 the config, or disable timeouts.".into())
        } else {
            Ok(())
        }
    } else {
        Err(
            "The default timeout config was removed, and no other config was put in its place."
                .into(),
        )
    }
}

/// Runtime plugin that registers the default identity cache implementation.
pub fn default_identity_cache_plugin() -> Option<SharedRuntimePlugin> {
    Some(
        default_plugin("default_identity_cache_plugin", |components| {
            components.with_identity_cache(Some(IdentityCache::lazy().build()))
        })
        .into_shared(),
    )
}

/// Runtime plugin that sets the default stalled stream protection config.
///
/// By default, when throughput falls below 1/Bs for more than 5 seconds, the
/// stream is cancelled.
#[deprecated(
    since = "1.2.0",
    note = "This function wasn't intended to be public, and didn't take the behavior major version as an argument, so it couldn't be evolved over time."
)]
pub fn default_stalled_stream_protection_config_plugin() -> Option<SharedRuntimePlugin> {
    #[allow(deprecated)]
    default_stalled_stream_protection_config_plugin_v2(BehaviorVersion::v2023_11_09())
}
fn default_stalled_stream_protection_config_plugin_v2(
    behavior_version: BehaviorVersion,
) -> Option<SharedRuntimePlugin> {
    Some(
        default_plugin(
            "default_stalled_stream_protection_config_plugin",
            |components| {
                components.with_config_validator(SharedConfigValidator::base_client_config_fn(
                    validate_stalled_stream_protection_config,
                ))
            },
        )
        .with_config(layer("default_stalled_stream_protection_config", |layer| {
            let mut config =
                StalledStreamProtectionConfig::enabled().grace_period(Duration::from_secs(5));
            // Before v2024_03_28, upload streams did not have stalled stream protection by default
            if !behavior_version.is_at_least(BehaviorVersion::v2024_03_28()) {
                config = config.upload_enabled(false);
            }
            layer.store_put(config.build());
        }))
        .into_shared(),
    )
}

fn enforce_content_length_runtime_plugin() -> Option<SharedRuntimePlugin> {
    Some(EnforceContentLengthRuntimePlugin::new().into_shared())
}

fn validate_stalled_stream_protection_config(
    components: &RuntimeComponentsBuilder,
    cfg: &ConfigBag,
) -> Result<(), BoxError> {
    if let Some(stalled_stream_protection_config) = cfg.load::<StalledStreamProtectionConfig>() {
        if stalled_stream_protection_config.is_enabled() {
            if components.sleep_impl().is_none() {
                return Err(
                    "An async sleep implementation is required for stalled stream protection to work. \
                     Please provide a `sleep_impl` on the config, or disable stalled stream protection.".into());
            }

            if components.time_source().is_none() {
                return Err(
                    "A time source is required for stalled stream protection to work.\
                     Please provide a `time_source` on the config, or disable stalled stream protection.".into());
            }
        }

        Ok(())
    } else {
        Err(
            "The default stalled stream protection config was removed, and no other config was put in its place."
                .into(),
        )
    }
}

/// Arguments for the [`default_plugins`] method.
///
/// This is a struct to enable adding new parameters in the future without breaking the API.
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct DefaultPluginParams {
    retry_partition_name: Option<Cow<'static, str>>,
    behavior_version: Option<BehaviorVersion>,
}

impl DefaultPluginParams {
    /// Creates a new [`DefaultPluginParams`].
    pub fn new() -> Self {
        Default::default()
    }

    /// Sets the retry partition name.
    pub fn with_retry_partition_name(mut self, name: impl Into<Cow<'static, str>>) -> Self {
        self.retry_partition_name = Some(name.into());
        self
    }

    /// Sets the behavior major version.
    pub fn with_behavior_version(mut self, version: BehaviorVersion) -> Self {
        self.behavior_version = Some(version);
        self
    }
}

/// All default plugins.
pub fn default_plugins(
    params: DefaultPluginParams,
) -> impl IntoIterator<Item = SharedRuntimePlugin> {
    let behavior_version = params
        .behavior_version
        .unwrap_or_else(BehaviorVersion::latest);

    [
        default_http_client_plugin(),
        default_identity_cache_plugin(),
        default_retry_config_plugin(
            params
                .retry_partition_name
                .expect("retry_partition_name is required"),
        ),
        default_sleep_impl_plugin(),
        default_time_source_plugin(),
        default_timeout_config_plugin(),
        enforce_content_length_runtime_plugin(),
        default_stalled_stream_protection_config_plugin_v2(behavior_version),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<SharedRuntimePlugin>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_smithy_runtime_api::client::runtime_plugin::RuntimePlugins;

    fn test_plugin_params(version: BehaviorVersion) -> DefaultPluginParams {
        DefaultPluginParams::new()
            .with_behavior_version(version)
            .with_retry_partition_name("dontcare")
    }
    fn config_for(plugins: impl IntoIterator<Item = SharedRuntimePlugin>) -> ConfigBag {
        let mut config = ConfigBag::base();
        let plugins = RuntimePlugins::new().with_client_plugins(plugins);
        plugins.apply_client_configuration(&mut config).unwrap();
        config
    }

    #[test]
    #[allow(deprecated)]
    fn v2024_03_28_stalled_stream_protection_difference() {
        let latest = config_for(default_plugins(test_plugin_params(
            BehaviorVersion::latest(),
        )));
        let v2023 = config_for(default_plugins(test_plugin_params(
            BehaviorVersion::v2023_11_09(),
        )));

        assert!(
            latest
                .load::<StalledStreamProtectionConfig>()
                .unwrap()
                .upload_enabled(),
            "stalled stream protection on uploads MUST be enabled after v2024_03_28"
        );
        assert!(
            !v2023
                .load::<StalledStreamProtectionConfig>()
                .unwrap()
                .upload_enabled(),
            "stalled stream protection on uploads MUST NOT be enabled before v2024_03_28"
        );
    }
}
