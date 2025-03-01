/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_smithy_types::config_bag::{Storable, StoreReplace};
use aws_smithy_types::retry::ErrorKind;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::trace;

const DEFAULT_CAPACITY: usize = 500;
const RETRY_COST: u32 = 5;
const RETRY_TIMEOUT_COST: u32 = RETRY_COST * 2;
const PERMIT_REGENERATION_AMOUNT: usize = 1;

/// Token bucket used for standard and adaptive retry.
#[derive(Clone, Debug)]
pub struct TokenBucket {
    semaphore: Arc<Semaphore>,
    max_permits: usize,
    timeout_retry_cost: u32,
    retry_cost: u32,
}

impl Storable for TokenBucket {
    type Storer = StoreReplace<Self>;
}

impl Default for TokenBucket {
    fn default() -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(DEFAULT_CAPACITY)),
            max_permits: DEFAULT_CAPACITY,
            timeout_retry_cost: RETRY_TIMEOUT_COST,
            retry_cost: RETRY_COST,
        }
    }
}

impl TokenBucket {
    /// Creates a new `TokenBucket` with the given initial quota.
    pub fn new(initial_quota: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(initial_quota)),
            max_permits: initial_quota,
            retry_cost: RETRY_COST,
            timeout_retry_cost: RETRY_TIMEOUT_COST,
        }
    }

    pub(crate) fn acquire(&self, err: &ErrorKind) -> Option<OwnedSemaphorePermit> {
        let retry_cost = if err == &ErrorKind::TransientError {
            self.timeout_retry_cost
        } else {
            self.retry_cost
        };

        self.semaphore
            .clone()
            .try_acquire_many_owned(retry_cost)
            .ok()
    }

    pub(crate) fn regenerate_a_token(&self) {
        if self.semaphore.available_permits() < (self.max_permits) {
            trace!("adding {PERMIT_REGENERATION_AMOUNT} back into the bucket");
            self.semaphore.add_permits(PERMIT_REGENERATION_AMOUNT)
        }
    }

    #[cfg(all(test, feature = "test-util"))]
    pub(crate) fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}
