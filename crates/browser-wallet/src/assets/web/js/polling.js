let pollingIntervals = {
    heartbeat: null,
    transaction: null,
    signing: null,
    network: null
};

// Request queue to prevent race conditions
const requestQueue = [];
let processingQueue = false;

// Add request to queue
function queueRequest(request) {
    requestQueue.push(request);
    if (!processingQueue) {
        processQueue();
    }
}

// Process queued requests
async function processQueue() {
    if (processingQueue || requestQueue.length === 0) return;
    
    processingQueue = true;
    
    while (requestQueue.length > 0) {
        const request = requestQueue.shift();
        try {
            await request();
        } catch (error) {
            console.error('Queue processing error:', error);
        }
    }
    
    processingQueue = false;
}

// Start heartbeat polling
function startHeartbeat() {
    pollingIntervals.heartbeat = setInterval(async () => {
        try {
            const response = await apiCall('api/heartbeat');
            if (!response || response.status !== 'alive') {
                console.error('Heartbeat failed:', response);
                showError('Lost connection to backend');
                stopAllPolling();
            }
        } catch (error) {
            console.error('Heartbeat error:', error);
            showError('Lost connection to backend');
            stopAllPolling();
        }
    }, 5000); // Every 5 seconds
}

// Poll for pending transactions with dynamic interval
function startTransactionPolling() {
    let pollDelay = 1000; // Start at 1 second
    const MAX_POLL_DELAY = 5000; // Max 5 seconds
    
    const poll = async () => {
        // Only poll if connected
        if (connectionState !== ConnectionState.CONNECTED) {
            console.log('Not polling transactions - not connected');
            pollingIntervals.transaction = null;
            return;
        }
        
        if (currentTransaction || isProcessingTransaction) {
            // Fast polling when processing
            pollDelay = 1000;
        } else {
            try {
                const tx = await apiCall('api/transaction/pending');
                if (tx && tx.id) {
                    pollDelay = 1000; // Reset to fast polling
                    queueRequest(() => processTransaction(tx));
                } else {
                    // Exponential backoff when no work
                    pollDelay = Math.min(pollDelay * 1.5, MAX_POLL_DELAY);
                }
            } catch (error) {
                console.error('Transaction polling error:', error);
                pollDelay = MAX_POLL_DELAY;
            }
        }
        
        pollingIntervals.transaction = setTimeout(poll, pollDelay);
    };
    
    poll();
}

// Poll for pending signing requests with dynamic interval
function startSigningPolling() {
    let pollDelay = 1000; // Start at 1 second
    const MAX_POLL_DELAY = 5000; // Max 5 seconds
    
    const poll = async () => {
        // Only poll if connected
        if (connectionState !== ConnectionState.CONNECTED) {
            console.log('Not polling signing requests - not connected');
            pollingIntervals.signing = null;
            return;
        }
        
        if (currentSigningRequest || isProcessingSigning) {
            // Fast polling when processing
            pollDelay = 1000;
        } else {
            try {
                const req = await apiCall('api/sign/pending');
                if (req && req.id) {
                    pollDelay = 1000; // Reset to fast polling
                    queueRequest(() => processSigningRequest(req));
                } else {
                    // Exponential backoff when no work
                    pollDelay = Math.min(pollDelay * 1.5, MAX_POLL_DELAY);
                }
            } catch (error) {
                console.error('Signing polling error:', error);
                pollDelay = MAX_POLL_DELAY;
            }
        }
        
        pollingIntervals.signing = setTimeout(poll, pollDelay);
    };
    
    poll();
}

// Check network details
async function checkNetworkDetails() {
    const details = await apiCall('api/network');
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
    Object.entries(pollingIntervals).forEach(([key, interval]) => {
        if (interval) {
            if (key === 'transaction' || key === 'signing') {
                clearTimeout(interval);
            } else {
                clearInterval(interval);
            }
        }
    });
    
    // Clear all intervals
    pollingIntervals = {
        heartbeat: null,
        transaction: null,
        signing: null,
        network: null
    };
}