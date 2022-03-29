//! Mines transactions

use crate::eth::pool::{transactions::PoolTransaction, Pool};
use std::{
    sync::Arc,
    task::{Context, Poll},
};

pub enum MiningMode {
    Instant,
    // TODO
}

impl MiningMode {
    pub fn poll(&self, pool: &Arc<Pool>, cx: &mut Context<'_>) -> Poll<Vec<PoolTransaction>> {
        todo!()
    }
}
