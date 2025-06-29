// Global initialization flag
let isInitialized = false;
let initializationPromise = null;

// Initialize application
async function initializeApp() {
    if (isInitialized) return;
    if (initializationPromise) return initializationPromise;
    
    initializationPromise = performInitialization();
    await initializationPromise;
    isInitialized = true;
}

async function performInitialization() {
    console.log('Starting wallet initialization...');
    
    // Set initial state
    setConnectionState(ConnectionState.INITIALIZING);
    updateStatus('Initializing...');
    
    // Start heartbeat immediately
    startHeartbeat();
    
    // Check network details
    checkNetworkDetails().catch(console.error);
    
    // Detect provider with retry logic
    const providerAvailable = await detectProvider();
    
    if (!providerAvailable) {
        setConnectionState(ConnectionState.PROVIDER_NOT_FOUND);
        updateStatus('No wallet detected');
        handleNoWalletDetected();
        return;
    }
    
    // Provider is available
    setConnectionState(ConnectionState.READY);
    updateStatus('Ready');
    
    // Set up event listeners
    setupWalletEventListeners();
    
    // Check for auto-reconnection
    await attemptAutoReconnection();
}

async function attemptAutoReconnection() {
    const lastConnection = connectionStorage.getLastConnection();
    
    if (!lastConnection || !lastConnection.address) {
        console.log('No previous connection found');
        return;
    }
    
    console.log('Attempting to reconnect to:', lastConnection.address);
    
    try {
        // Check if account is still available
        const accounts = await window.ethereum.request({ method: 'eth_accounts' });
        
        if (accounts.includes(lastConnection.address)) {
            // Account is available, restore connection
            console.log('Restoring previous connection');
            
            connectedAccount = lastConnection.address;
            updateAccount(connectedAccount);
            setConnectionState(ConnectionState.CONNECTED);
            updateStatus('Connected');
            
            // Hide connect button
            const connectContainer = document.getElementById('connect-container');
            if (connectContainer) {
                connectContainer.style.display = 'none';
            }
            
            // Get current chain ID
            const chainId = await window.ethereum.request({ method: 'eth_chainId' });
            const chainIdDecimal = parseInt(chainId, 16);
            
            // Save updated connection info
            connectionStorage.saveConnection(connectedAccount, chainIdDecimal);
            
            // Report to backend
            await apiCall('api/account', {
                method: 'POST',
                body: JSON.stringify({ 
                    address: connectedAccount,
                    chain_id: chainIdDecimal
                })
            });
            
            // Start polling
            startTransactionPolling();
            startSigningPolling();
            
        } else {
            // Account not available, clear stored connection
            console.log('Previous account not available, clearing connection');
            connectionStorage.clearConnection();
        }
    } catch (error) {
        console.error('Auto-reconnection failed:', error);
        connectionStorage.clearConnection();
    }
}

function handleNoWalletDetected() {
    const connectButton = document.getElementById('connect-button');
    if (connectButton) {
        connectButton.textContent = 'Install Wallet';
        connectButton.disabled = true;
    }
    
    // Continue checking for wallet injection
    const checkInterval = setInterval(async () => {
        if (window.ethereum) {
            clearInterval(checkInterval);
            console.log('Wallet detected after page load');
            
            setConnectionState(ConnectionState.READY);
            updateStatus('Ready');
            
            const connectButton = document.getElementById('connect-button');
            if (connectButton) {
                connectButton.textContent = 'Connect Wallet';
                connectButton.disabled = false;
            }
            
            setupWalletEventListeners();
            
            // Check for auto-reconnection
            await attemptAutoReconnection();
        }
    }, 1000);
}

// Set up wallet event listeners
function setupWalletEventListeners() {
    if (!window.ethereum) return;
    
    // Remove any existing listeners to avoid duplicates
    window.ethereum.removeAllListeners('accountsChanged');
    window.ethereum.removeAllListeners('chainChanged');
    
    window.ethereum.on('accountsChanged', async (accounts) => {
        if (accounts.length === 0) {
            // Disconnected
            connectedAccount = null;
            updateAccount(null);
            setConnectionState(ConnectionState.DISCONNECTED);
            updateStatus('Disconnected');
            connectionStorage.clearConnection();
            
            // Show connect button
            const connectContainer = document.getElementById('connect-container');
            if (connectContainer) {
                connectContainer.style.display = 'block';
            }
            
            // Report to backend
            await apiCall('api/account', {
                method: 'POST',
                body: JSON.stringify({ address: null })
            });
            
            // Stop polling
            if (pollingIntervals.transaction) {
                clearInterval(pollingIntervals.transaction);
                pollingIntervals.transaction = null;
            }
            if (pollingIntervals.signing) {
                clearInterval(pollingIntervals.signing);
                pollingIntervals.signing = null;
            }
        } else {
            // Account changed
            connectedAccount = accounts[0];
            updateAccount(connectedAccount);
            setConnectionState(ConnectionState.CONNECTED);
            updateStatus('Connected');
            
            // Hide connect button
            const connectContainer = document.getElementById('connect-container');
            if (connectContainer) {
                connectContainer.style.display = 'none';
            }
            
            // Get chain ID
            const chainId = await window.ethereum.request({ method: 'eth_chainId' });
            const chainIdDecimal = parseInt(chainId, 16);
            
            // Save connection
            connectionStorage.saveConnection(connectedAccount, chainIdDecimal);
            
            // Report to backend
            await apiCall('api/account', {
                method: 'POST',
                body: JSON.stringify({ 
                    address: connectedAccount,
                    chain_id: chainIdDecimal
                })
            });
            
            // Start polling if not already
            if (!pollingIntervals.transaction) {
                startTransactionPolling();
            }
            if (!pollingIntervals.signing) {
                startSigningPolling();
            }
        }
    });
    
    // Listen for chain changes - secure handling without reload
    window.ethereum.on('chainChanged', async (chainId) => {
        const chainIdDecimal = parseInt(chainId, 16);
        console.log('Chain changed to:', chainIdDecimal);
        
        // Cancel all pending operations
        if (currentTransaction) {
            console.warn('Chain changed while transaction pending - cancelling');
            // Report transaction failure
            await apiCall('api/transaction/response', {
                method: 'POST',
                body: JSON.stringify({
                    id: currentTransaction.id,
                    status: 'error',
                    error: 'Network changed during transaction'
                })
            });
            currentTransaction = null;
            isProcessingTransaction = false;
        }
        
        if (currentSigningRequest) {
            console.warn('Chain changed while signing pending - cancelling');
            // Report signing failure
            await apiCall('api/sign/response', {
                method: 'POST',
                body: JSON.stringify({
                    id: currentSigningRequest.id,
                    status: 'error',
                    error: 'Network changed during signing'
                })
            });
            currentSigningRequest = null;
            isProcessingSigning = false;
        }
        
        // Update saved connection if connected
        if (connectedAccount) {
            connectionStorage.saveConnection(connectedAccount, chainIdDecimal);
        }
        
        // Update backend with new chain ID
        await apiCall('api/account', {
            method: 'POST',
            body: JSON.stringify({ 
                address: connectedAccount,
                chain_id: chainIdDecimal
            })
        });
        
        // Update UI
        updateStatus(`Chain changed to ${chainIdDecimal}`);
        
        // Show warning
        const warningDiv = document.createElement('div');
        warningDiv.className = 'security-warning';
        warningDiv.style.position = 'fixed';
        warningDiv.style.top = '20px';
        warningDiv.style.left = '50%';
        warningDiv.style.transform = 'translateX(-50%)';
        warningDiv.style.zIndex = '1000';
        warningDiv.innerHTML = '\u26a0\ufe0f Network changed. Please verify you are on the correct network.';
        document.body.appendChild(warningDiv);
        
        // Remove warning after 5 seconds
        setTimeout(() => {
            warningDiv.remove();
        }, 5000);
        
        // Re-check network details
        checkNetworkDetails().catch(console.error);
    });
}

// Initialize on page load
window.addEventListener('DOMContentLoaded', async () => {
    await initializeApp();
});

// Cleanup on page unload
window.addEventListener('beforeunload', () => {
    stopAllPolling();
});

// Listen for connection state changes to update UI
onConnectionStateChange((newState, oldState) => {
    // Update connect button based on state
    const connectButton = document.getElementById('connect-button');
    if (!connectButton) return;
    
    switch (newState) {
        case ConnectionState.INITIALIZING:
            connectButton.disabled = true;
            connectButton.textContent = 'Initializing...';
            break;
        case ConnectionState.PROVIDER_NOT_FOUND:
            connectButton.disabled = true;
            connectButton.textContent = 'Install Wallet';
            break;
        case ConnectionState.READY:
            connectButton.disabled = false;
            connectButton.textContent = 'Connect Wallet';
            break;
        case ConnectionState.CONNECTING:
            connectButton.disabled = true;
            connectButton.textContent = 'Connecting...';
            break;
        case ConnectionState.CONNECTED:
            connectButton.disabled = false;
            connectButton.textContent = 'Connected';
            break;
        case ConnectionState.DISCONNECTED:
            connectButton.disabled = false;
            connectButton.textContent = 'Connect Wallet';
            break;
        case ConnectionState.ERROR:
            connectButton.disabled = false;
            connectButton.textContent = 'Retry Connection';
            break;
    }
});