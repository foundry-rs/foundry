/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Newtypes for endpoint-related parameters
//!
//! Parameters require newtypes so they have distinct types when stored in layers in config bag.

use aws_smithy_types::config_bag::{Storable, StoreReplace};

/// Newtype for `use_fips`
#[derive(Clone, Debug)]
pub struct UseFips(pub bool);
impl Storable for UseFips {
    type Storer = StoreReplace<UseFips>;
}

/// Newtype for `use_dual_stack`
#[derive(Clone, Debug)]
pub struct UseDualStack(pub bool);
impl Storable for UseDualStack {
    type Storer = StoreReplace<UseDualStack>;
}

/// Newtype for `endpoint_url`
#[derive(Clone, Debug)]
pub struct EndpointUrl(pub String);
impl Storable for EndpointUrl {
    type Storer = StoreReplace<EndpointUrl>;
}
