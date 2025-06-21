let pollingIntervals = {
    heartbeat: null,
    transaction: null,
    signing: null,
    network: null
};

// Start heartbeat polling
function startHeartbeat() {
    pollingIntervals.heartbeat = setInterval(async () => {
        const response = await apiCall('heartbeat');
        if (!response) {
            showError('Lost connection to backend');
            stopAllPolling();
        }
    }, 5000); // Every 5 seconds
}

// Poll for pending transactions
function startTransactionPolling() {
    pollingIntervals.transaction = setInterval(async () => {
        if (currentTransaction || isProcessingTransaction) return; // Already processing one
        
        const tx = await apiCall('get_pending_transaction');
        if (tx && tx.id) {
            await processTransaction(tx);
        }
    }, 1000); // Every 1 second
}

// Poll for pending signing requests
function startSigningPolling() {
    pollingIntervals.signing = setInterval(async () => {
        if (currentSigningRequest || isProcessingSigning) return; // Already processing one
        
        const req = await apiCall('get_pending_signing');
        if (req && req.id) {
            await processSigningRequest(req);
        }
    }, 1000); // Every 1 second
}

// Check network details
async function checkNetworkDetails() {
    const details = await apiCall('get_boa_network_details');
    if (details) {
        updateNetwork(details.network_name);
        
        // Verify we're on the right network
        if (window.ethereum && connectedAccount) {
            try {
                const chainId = await window.ethereum.request({ method: 'eth_chainId' });
                const currentChainId = parseInt(chainId, 16);
                
                if (currentChainId !== details.chain_id) {
                    showError(`Wrong network! Please switch to ${details.network_name} (Chain ID: ${details.chain_id})`);
                }
            } catch (error) {
                console.error('Failed to check chain ID:', error);
            }
        }
    }
}

// Stop all polling
function stopAllPolling() {
    Object.values(pollingIntervals).forEach(interval => {
        if (interval) clearInterval(interval);
    });
}