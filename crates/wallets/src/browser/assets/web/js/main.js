// Initialize on page load
window.addEventListener('DOMContentLoaded', async () => {
    // Initialize UI immediately
    updateStatus('Initializing...');
    
    // Start heartbeat
    startHeartbeat();
    
    // Check network details asynchronously
    checkNetworkDetails().catch(console.error);
    
    // Check for wallet availability after a short delay
    setTimeout(async () => {
        if (window.ethereum) {
            // Wallet is available
            updateStatus('Ready');
            
            // Set up event listeners
            setupWalletEventListeners();
            
            // Check if already connected (but don't auto-connect)
            try {
                const accounts = await window.ethereum.request({ method: 'eth_accounts' });
                if (accounts.length > 0) {
                    // Already connected from previous session
                    connectedAccount = accounts[0];
                    updateAccount(connectedAccount);
                    updateStatus('Connected');
                    
                    // Hide connect button
                    const connectContainer = document.getElementById('connect-container');
                    if (connectContainer) {
                        connectContainer.style.display = 'none';
                    }
                    
                    // Reset connect button state in case it was left in connecting state
                    const connectButton = document.getElementById('connect-button');
                    if (connectButton) {
                        connectButton.disabled = false;
                        connectButton.textContent = 'Connect Wallet';
                    }
                    
                    // Report to backend
                    await apiCall('update_account_status', {
                        method: 'POST',
                        body: JSON.stringify({ address: connectedAccount })
                    });
                    
                    // Start polling
                    startTransactionPolling();
                    startSigningPolling();
                }
            } catch (error) {
                console.error('Error checking existing connection:', error);
            }
        } else {
            // No wallet detected
            updateStatus('No wallet detected');
            const connectButton = document.getElementById('connect-button');
            if (connectButton) {
                connectButton.textContent = 'Install Wallet';
                connectButton.disabled = true;
            }
            
            // Check again periodically in case wallet gets injected later
            const checkInterval = setInterval(() => {
                if (window.ethereum) {
                    clearInterval(checkInterval);
                    updateStatus('Ready');
                    const connectButton = document.getElementById('connect-button');
                    if (connectButton) {
                        connectButton.textContent = 'Connect Wallet';
                        connectButton.disabled = false;
                    }
                    setupWalletEventListeners();
                }
            }, 1000);
        }
    }, 100); // Small delay to ensure DOM is ready
});

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
            updateStatus('Disconnected');
            
            // Show connect button
            const connectContainer = document.getElementById('connect-container');
            if (connectContainer) {
                connectContainer.style.display = 'block';
            }
            
            // Report to backend
            await apiCall('update_account_status', {
                method: 'POST',
                body: JSON.stringify({ address: null })
            });
            
            // Stop polling
            if (pollingIntervals.transaction) {
                clearInterval(pollingIntervals.transaction);
                pollingIntervals.transaction = null;
            }
        } else {
            // Account changed
            connectedAccount = accounts[0];
            updateAccount(connectedAccount);
            updateStatus('Connected');
            
            // Hide connect button
            const connectContainer = document.getElementById('connect-container');
            if (connectContainer) {
                connectContainer.style.display = 'none';
            }
            
            // Report to backend
            await apiCall('update_account_status', {
                method: 'POST',
                body: JSON.stringify({ address: connectedAccount })
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
    
    // Listen for chain changes
    window.ethereum.on('chainChanged', (chainId) => {
        // Reload to ensure consistent state
        window.location.reload();
    });
}

// Cleanup on page unload
window.addEventListener('beforeunload', () => {
    stopAllPolling();
});