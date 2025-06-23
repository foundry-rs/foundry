let currentTransaction = null;
let currentSigningRequest = null;
let connectedAccount = null;
let isProcessingTransaction = false;
let isProcessingSigning = false;

// Track processed transactions to prevent replay attacks
const processedTransactions = new Set();
const processedSigningRequests = new Set();

// Check if MetaMask is available
function isMetaMaskAvailable() {
    return typeof window.ethereum !== 'undefined';
}

// Validate wallet connection integrity
async function validateConnection() {
    if (!window.ethereum) return false;
    if (connectionState !== ConnectionState.CONNECTED) return false;
    
    try {
        // Check if wallet is still connected
        const accounts = await window.ethereum.request({ 
            method: 'eth_accounts' 
        });
        
        // Verify account matches what we expect
        if (accounts.length === 0 || accounts[0] !== connectedAccount) {
            // Account mismatch - potential security issue
            await handleSecurityError('Wallet connection changed unexpectedly');
            return false;
        }
        
        // Verify chain ID hasn't changed unexpectedly
        const chainId = await window.ethereum.request({ method: 'eth_chainId' });
        const currentChainId = parseInt(chainId, 16);
        
        // Update saved connection info
        connectionStorage.saveConnection(connectedAccount, currentChainId);
        
        // Report current state to backend
        await apiCall('api/account', {
            method: 'POST',
            body: JSON.stringify({ 
                address: connectedAccount,
                chain_id: currentChainId
            })
        });
        
        return true;
    } catch (error) {
        console.error('Connection validation failed:', error);
        setConnectionState(ConnectionState.ERROR);
        return false;
    }
}

// Handle security errors
async function handleSecurityError(message) {
    console.error('Security error:', message);
    
    // Clear connection
    connectedAccount = null;
    updateAccount(null);
    setConnectionState(ConnectionState.ERROR);
    updateStatus('Disconnected - Security Error');
    connectionStorage.clearConnection();
    
    // Stop all operations
    stopAllPolling();
    
    // Show error
    showError(message);
    
    // Report to backend
    await apiCall('api/account', {
        method: 'POST',
        body: JSON.stringify({ address: null })
    });
}

// Connect to MetaMask
async function connectWallet() {
    if (!isMetaMaskAvailable()) {
        showError('MetaMask is not installed! Please install MetaMask to continue.');
        return;
    }

    // Prevent concurrent connection attempts
    if (connectionState === ConnectionState.CONNECTING) {
        console.log('Connection already in progress');
        return;
    }

    const connectButton = document.getElementById('connect-button');
    
    try {
        setConnectionState(ConnectionState.CONNECTING);
        
        if (connectButton) {
            connectButton.disabled = true;
            connectButton.textContent = 'Connecting...';
        }
        
        // Update status to show we're connecting
        updateStatus('Connecting...');

        // First check if we already have permission (wallet might already be connected)
        const existingAccounts = await window.ethereum.request({ 
            method: 'eth_accounts' 
        });

        let accounts;
        if (existingAccounts && existingAccounts.length > 0) {
            // Already have permission, use existing accounts
            accounts = existingAccounts;
        } else {
            // Need to request permission
            accounts = await window.ethereum.request({ 
                method: 'eth_requestAccounts' 
            });
        }
        
        if (accounts.length > 0) {
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
            
            // Save connection to localStorage
            connectionStorage.saveConnection(connectedAccount, chainIdDecimal);
            
            // Report to backend
            await apiCall('api/account', {
                method: 'POST',
                body: JSON.stringify({ 
                    address: connectedAccount,
                    chain_id: chainIdDecimal
                })
            });
            
            // Start polling for transactions and signing requests
            startTransactionPolling();
            startSigningPolling();
        } else {
            // No accounts returned, reset button and status
            setConnectionState(ConnectionState.READY);
            updateStatus('Ready');
            if (connectButton) {
                connectButton.disabled = false;
                connectButton.textContent = 'Connect Wallet';
            }
        }
    } catch (error) {
        console.error('Failed to connect:', error);
        
        // Reset status
        setConnectionState(ConnectionState.READY);
        updateStatus('Ready');
        
        // Don't show error for user rejection
        if (error.code !== 4001) {
            showError(`Failed to connect: ${error.message}`);
        }
        
        // Always reset button state on error
        if (connectButton) {
            connectButton.disabled = false;
            connectButton.textContent = 'Connect Wallet';
        }
    }
}

// Process pending transaction
async function processTransaction(tx) {
    if (isProcessingTransaction) return;
    
    // Prevent replay attacks
    if (processedTransactions.has(tx.id)) {
        console.error('Transaction already processed:', tx.id);
        return;
    }
    
    // Validate connection before processing
    if (!await validateConnection()) {
        console.error('Connection validation failed');
        return;
    }
    
    console.log('Received transaction:', tx);
    
    // The transaction comes directly with the fields, not nested
    const txData = tx;
    
    // Parse chain ID - it's coming as hex string "0x7a69"
    let chainId = parseInt(txData.chainId, 16);
    
    // Normalize the transaction data for consistent access
    // Keep hex values as-is since they're already in the correct format
    currentTransaction = {
        id: tx.id,
        from: txData.from,
        to: txData.to,
        value: txData.value || '0x0',
        data: txData.data || txData.input || '0x',
        gas: txData.gas,
        gas_price: txData.gasPrice,
        max_fee_per_gas: txData.maxFeePerGas,
        max_priority_fee_per_gas: txData.maxPriorityFeePerGas,
        nonce: txData.nonce,
        chain_id: chainId
    };
    
    // Ensure data field doesn't get double-prefixed
    if (currentTransaction.data === '0x0x') {
        currentTransaction.data = '0x';
    }
    
    console.log('Normalized transaction:', currentTransaction);
    
    isProcessingTransaction = true;
    processedTransactions.add(tx.id);
    
    // Set timeout to clean up old transaction IDs after 5 minutes
    setTimeout(() => {
        processedTransactions.delete(tx.id);
    }, 300000);
    
    // Display transaction details with security warnings
    const details = document.getElementById('transaction-details');
    if (details) {
        // Security warning for contract interactions
        let securityWarning = '';
        if (currentTransaction.data && currentTransaction.data !== '0x') {
            securityWarning = '<div class="security-warning">⚠️ This transaction interacts with a smart contract</div>';
        }
        if (!currentTransaction.to) {
            securityWarning = '<div class="security-warning">⚠️ This transaction deploys a new contract</div>';
        }
        
        let html = securityWarning;
        html += '<div class="tx-row"><span class="tx-label">From</span><span class="tx-value address">' + currentTransaction.from + '</span></div>';
        html += '<div class="tx-row"><span class="tx-label">To</span><span class="tx-value address">' + (currentTransaction.to || 'Contract Creation') + '</span></div>';
        html += '<div class="tx-row"><span class="tx-label">Value</span><span class="tx-value">' + formatWei(currentTransaction.value || '0') + ' ETH</span></div>';
        
        if (currentTransaction.gas) {
            html += '<div class="tx-row"><span class="tx-label">Gas Limit</span><span class="tx-value">' + currentTransaction.gas + '</span></div>';
        }
        
        if (currentTransaction.data && currentTransaction.data !== '0x') {
            const displayData = currentTransaction.data.length > 66 ? currentTransaction.data.substring(0, 66) + '...' : currentTransaction.data;
            html += '<div class="tx-row"><span class="tx-label">Data</span><span class="tx-value" style="font-size: 0.75rem;">' + displayData + '</span></div>';
        }
        
        details.innerHTML = html;
    }
    
    // Show transaction container
    const txContainer = document.getElementById('transaction-container');
    if (txContainer) {
        txContainer.style.display = 'block';
    }
    
    // Reset status
    const statusEl = document.getElementById('transaction-status');
    if (statusEl) {
        statusEl.innerHTML = '';
    }
}

// Update transaction status
function updateTransactionStatus(message, isError = false) {
    const statusEl = document.getElementById('transaction-status');
    if (statusEl) {
        statusEl.innerHTML = `
            <div class="${isError ? 'error-message' : 'detecting'}" style="margin-top: 1rem;">
                ${!isError ? '<div class="spinner"></div> ' : ''}${message}
            </div>
        `;
    }
}

// Approve transaction
async function approveTransaction() {
    if (!currentTransaction) return;
    
    const actionsEl = document.getElementById('transaction-actions');
    if (actionsEl) {
        actionsEl.style.display = 'none';
    }
    
    try {
        // Check if we need to switch chains
        const chainId = await window.ethereum.request({ method: 'eth_chainId' });
        const currentChainId = parseInt(chainId, 16);
        
        if (currentChainId !== currentTransaction.chain_id) {
            updateTransactionStatus('Switching to correct network...');
            
            console.log('Current chain ID:', currentChainId, 'Transaction chain ID:', currentTransaction.chain_id);
            
            try {
                // Ensure chain_id is a number before converting to hex
                const targetChainId = typeof currentTransaction.chain_id === 'string' 
                    ? parseInt(currentTransaction.chain_id) 
                    : currentTransaction.chain_id;
                const hexChainId = '0x' + targetChainId.toString(16);
                
                console.log('Switching to chain:', hexChainId);
                
                await window.ethereum.request({
                    method: 'wallet_switchEthereumChain',
                    params: [{ chainId: hexChainId }],
                });
            } catch (switchError) {
                console.error('Switch network error:', switchError);
                // This error code indicates that the chain has not been added to MetaMask
                if (switchError.code === 4902) {
                    throw new Error('Please add this network to MetaMask first');
                } else {
                    throw new Error('Failed to switch network. Please switch manually and try again.');
                }
            }
        }
        
        updateTransactionStatus('Please sign the transaction in MetaMask...');
        
        // Build transaction parameters
        // Values are already in hex format from the server
        const txParams = {
            from: currentTransaction.from,
            to: currentTransaction.to || undefined,
            value: currentTransaction.value || '0x0',
            data: currentTransaction.data || '0x',
        };
        
        // Ensure we don't have empty strings that would bypass the || operator
        if (txParams.value === '') {
            txParams.value = '0x0';
        }
        if (txParams.data === '') {
            txParams.data = '0x';
        }
        
        // Prevent any accidental double hex prefixing
        const ensureHexPrefix = (value) => {
            if (!value) return '0x0';
            // If it already starts with 0x, return as-is
            if (value.startsWith('0x')) return value;
            // Otherwise add 0x prefix
            return '0x' + value;
        };
        
        // Clean up any potential double prefixes
        if (txParams.value && txParams.value.startsWith('0x0x')) {
            console.warn('Cleaning double hex prefix from value:', txParams.value);
            txParams.value = '0x' + txParams.value.substring(4);
        }
        if (txParams.data && txParams.data.startsWith('0x0x')) {
            console.warn('Cleaning double hex prefix from data:', txParams.data);
            txParams.data = '0x' + txParams.data.substring(4);
        }
        
        console.log('Final transaction params before sending:', txParams);
        
        if (currentTransaction.gas) {
            txParams.gas = currentTransaction.gas;
        }
        
        if (currentTransaction.gas_price) {
            txParams.gasPrice = currentTransaction.gas_price;
        }
        
        if (currentTransaction.max_fee_per_gas) {
            txParams.maxFeePerGas = currentTransaction.max_fee_per_gas;
        }
        
        if (currentTransaction.max_priority_fee_per_gas) {
            txParams.maxPriorityFeePerGas = currentTransaction.max_priority_fee_per_gas;
        }
        
        if (currentTransaction.nonce !== null && currentTransaction.nonce !== undefined) {
            txParams.nonce = currentTransaction.nonce;
        }
        
        // Send transaction
        const txHash = await window.ethereum.request({
            method: 'eth_sendTransaction',
            params: [txParams]
        });
        
        updateTransactionStatus('Transaction sent! Notifying Foundry...');
        
        // Report success
        await apiCall('api/transaction/response', {
            method: 'POST',
            body: JSON.stringify({
                id: currentTransaction.id,
                status: 'success',
                hash: txHash
            })
        });
        
        // Show success message
        const txContainer = document.getElementById('transaction-container');
        if (txContainer) {
            txContainer.innerHTML = `
                <div class="success-message">
                    ✅ Transaction sent
                    <br>
                    <span style="font-size: 0.75rem; opacity: 0.8;">${txHash}</span>
                </div>
            `;
        }
        
        // Reset state for next transaction
        currentTransaction = null;
        isProcessingTransaction = false;
        
    } catch (error) {
        console.error('Transaction failed:', error);
        
        // Report error
        await apiCall('api/transaction/response', {
            method: 'POST',
            body: JSON.stringify({
                id: currentTransaction.id,
                status: 'error',
                error: error.message || 'Transaction failed'
            })
        });
        
        // Show error as final state
        const txContainer = document.getElementById('transaction-container');
        if (txContainer) {
            let errorMessage = error.message || 'Transaction failed';
            
            // Handle common error cases with clearer messages
            if (error.code === 4001) {
                errorMessage = 'Transaction rejected by user';
            } else if (error.code === -32002) {
                errorMessage = 'Request already pending. Please check your wallet.';
            } else if (error.code === -32603) {
                errorMessage = 'Internal wallet error. Please try again.';
            }
            
            txContainer.innerHTML = `
                <div class="error-message">
                    ❌ ${errorMessage}
                </div>
            `;
        }
        
        // Reset state
        currentTransaction = null;
        isProcessingTransaction = false;
    }
}

// Reject transaction
async function rejectTransaction() {
    if (!currentTransaction) return;
    
    const actionsEl = document.getElementById('transaction-actions');
    if (actionsEl) {
        actionsEl.style.display = 'none';
    }
    
    updateTransactionStatus('Rejecting transaction...');
    
    await apiCall('api/transaction/response', {
        method: 'POST',
        body: JSON.stringify({
            id: currentTransaction.id,
            status: 'error',
            error: 'User rejected transaction'
        })
    });
    
    // Show rejection message as final state
    const txContainer = document.getElementById('transaction-container');
    if (txContainer) {
        txContainer.innerHTML = `
            <div class="error-message">
                ❌ Transaction rejected
            </div>
        `;
    }
    
    // Reset state
    currentTransaction = null;
    isProcessingTransaction = false;
    
    // Don't hide the container - leave it as end state
}

// Process signing request
async function processSigningRequest(req) {
    if (isProcessingSigning) return;
    
    // Prevent replay attacks
    if (processedSigningRequests.has(req.id)) {
        console.error('Signing request already processed:', req.id);
        return;
    }
    
    // Validate connection before processing
    if (!await validateConnection()) {
        console.error('Connection validation failed');
        return;
    }
    
    currentSigningRequest = req;
    isProcessingSigning = true;
    processedSigningRequests.add(req.id);
    
    // Set timeout to clean up old request IDs after 5 minutes
    setTimeout(() => {
        processedSigningRequests.delete(req.id);
    }, 300000);
    
    try {
        let signature;
        
        // Handle both old format (string) and new format (enum serialized as snake_case)
        const signType = req.sign_type || req.type;
        if (signType === 'personal_sign' || signType === 'PersonalSign') {
            // Personal sign - eth_sign equivalent
            signature = await window.ethereum.request({
                method: 'personal_sign',
                params: [req.message || req.data, connectedAccount]
            });
        } else if (signType === 'sign_typed_data' || signType === 'SignTypedData') {
            // EIP-712 typed data signing
            const typedData = JSON.parse(req.message || req.data);
            signature = await window.ethereum.request({
                method: 'eth_signTypedData_v4',
                params: [connectedAccount, req.message || req.data]
            });
        } else {
            throw new Error(`Unknown signing type: ${signType}`);
        }
        
        // Report success
        await apiCall('api/sign/response', {
            method: 'POST',
            body: JSON.stringify({
                id: currentSigningRequest.id,
                status: 'success',
                signature: signature
            })
        });
        
        // Show success in UI
        showSigningSuccess(signType);
        
    } catch (error) {
        console.error('Signing failed:', error);
        
        // Report error
        await apiCall('api/sign/response', {
            method: 'POST',
            body: JSON.stringify({
                id: currentSigningRequest.id,
                status: 'error',
                error: error.message || 'Signing failed'
            })
        });
        
        // Show error in UI
        showSigningError(error.message || 'Signing failed');
    }
    
    // Reset state
    currentSigningRequest = null;
    isProcessingSigning = false;
}

// Show signing success
function showSigningSuccess(type) {
    const txContainer = document.getElementById('transaction-container');
    if (txContainer) {
        const messageType = (type === 'personal_sign' || type === 'PersonalSign') ? 'Message' : 'Typed data';
        txContainer.innerHTML = `
            <div class="success-message">
                ✅ ${messageType} signed successfully
            </div>
        `;
        txContainer.style.display = 'block';
    }
}

// Show signing error
function showSigningError(error) {
    const txContainer = document.getElementById('transaction-container');
    if (txContainer) {
        txContainer.innerHTML = `
            <div class="error-message">
                ❌ ${error}
            </div>
        `;
        txContainer.style.display = 'block';
    }
}