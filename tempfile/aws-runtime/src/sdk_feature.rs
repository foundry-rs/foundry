/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_smithy_types::config_bag::{Storable, StoreAppend};

/// IDs for the features that may be used in the AWS SDK
#[non_exhaustive]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AwsSdkFeature {
    /// Indicates that an operation was called by the S3 Transfer Manager
    S3Transfer,
}

impl Storable for AwsSdkFeature {
    type Storer = StoreAppend<Self>;
}
