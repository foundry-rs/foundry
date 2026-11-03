/// A `info!` helper macro that emits to the target, the node logger listens for
macro_rules! node_info {
    ($($arg:tt)*) => {
        {
            let _guard = $crate::logging::NODE_LOG_LOCK.lock();
            tracing::info!(target: $crate::logging::NODE_USER_LOG_TARGET, $($arg)*);
        }
    };
}

pub(crate) use node_info;
