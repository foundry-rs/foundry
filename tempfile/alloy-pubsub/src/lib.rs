#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

mod connect;
pub use connect::PubSubConnect;

mod frontend;
pub use frontend::PubSubFrontend;

mod ix;

mod handle;
pub use handle::{ConnectionHandle, ConnectionInterface};

mod managers;

mod service;

mod sub;
pub use sub::{
    RawSubscription, SubAnyStream, SubResultStream, Subscription, SubscriptionItem,
    SubscriptionStream,
};
