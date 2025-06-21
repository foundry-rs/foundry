let currentTransaction = null;
let currentSigningRequest = null;
let connectedAccount = null;
let isProcessingTransaction = false;
let isProcessingSigning = false;

// Check if MetaMask is available
function isMetaMaskAvailable() {
    return typeof window.ethereum !== 'undefined';
}

// Connect to MetaMask
async function connectWallet() {
    if (!isMetaMaskAvailable()) {
        showError('MetaMask is not installed! Please install MetaMask to continue.');
        return;
    }

    const connectButton = document.getElementById('connect-button');
    
    try {
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
            
            // Start polling for transactions and signing requests
            startTransactionPolling();
            startSigningPolling();
        } else {
            // No accounts returned, reset button and status
            updateStatus('Ready');
            if (connectButton) {
                connectButton.disabled = false;
                connectButton.textContent = 'Connect Wallet';
            }
        }
    } catch (error) {
        console.error('Failed to connect:', error);
        
        // Reset status
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
    
    currentTransaction = tx;
    isProcessingTransaction = true;
    
    // Display transaction details
    const details = document.getElementById('transaction-details');
    if (details) {
        let html = '<div class="tx-row"><span class="tx-label">From</span><span class="tx-value address">' + tx.from + '</span></div>';
        html += '<div class="tx-row"><span class="tx-label">To</span><span class="tx-value address">' + (tx.to || 'Contract Creation') + '</span></div>';
        html += '<div class="tx-row"><span class="tx-label">Value</span><span class="tx-value">' + formatWei(tx.value || '0') + '</span></div>';
        
        if (tx.gas) {
            html += '<div class="tx-row"><span class="tx-label">Gas Limit</span><span class="tx-value">' + tx.gas + '</span></div>';
        }
        
        if (tx.data && tx.data !== '0x') {
            const displayData = tx.data.length > 66 ? tx.data.substring(0, 66) + '...' : tx.data;
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
            
            try {
                await window.ethereum.request({
                    method: 'wallet_switchEthereumChain',
                    params: [{ chainId: '0x' + currentTransaction.chain_id.toString(16) }],
                });
            } catch (switchError) {
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
        const txParams = {
            from: currentTransaction.from,
            to: currentTransaction.to || undefined,
            value: '0x' + BigInt(currentTransaction.value || '0').toString(16),
            data: currentTransaction.data || '0x',
        };
        
        if (currentTransaction.gas) {
            txParams.gas = '0x' + BigInt(currentTransaction.gas).toString(16);
        }
        
        if (currentTransaction.gas_price) {
            txParams.gasPrice = '0x' + BigInt(currentTransaction.gas_price).toString(16);
        }
        
        if (currentTransaction.max_fee_per_gas) {
            txParams.maxFeePerGas = '0x' + BigInt(currentTransaction.max_fee_per_gas).toString(16);
        }
        
        if (currentTransaction.max_priority_fee_per_gas) {
            txParams.maxPriorityFeePerGas = '0x' + BigInt(currentTransaction.max_priority_fee_per_gas).toString(16);
        }
        
        if (currentTransaction.nonce !== null && currentTransaction.nonce !== undefined) {
            txParams.nonce = '0x' + currentTransaction.nonce.toString(16);
        }
        
        // Send transaction
        const txHash = await window.ethereum.request({
            method: 'eth_sendTransaction',
            params: [txParams]
        });
        
        updateTransactionStatus('Transaction sent! Notifying Foundry...');
        
        // Report success
        await apiCall('report_transaction_result', {
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
        await apiCall('report_transaction_result', {
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
    
    await apiCall('report_transaction_result', {
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
    
    currentSigningRequest = req;
    isProcessingSigning = true;
    
    try {
        let signature;
        
        if (req.type === 'personal_sign') {
            // Personal sign - eth_sign equivalent
            signature = await window.ethereum.request({
                method: 'personal_sign',
                params: [req.data, connectedAccount]
            });
        } else if (req.type === 'sign_typed_data') {
            // EIP-712 typed data signing
            const typedData = JSON.parse(req.data);
            signature = await window.ethereum.request({
                method: 'eth_signTypedData_v4',
                params: [connectedAccount, req.data]
            });
        } else {
            throw new Error(`Unknown signing type: ${req.type}`);
        }
        
        // Report success
        await apiCall('report_signing_result', {
            method: 'POST',
            body: JSON.stringify({
                id: currentSigningRequest.id,
                status: 'success',
                signature: signature
            })
        });
        
        // Show success in UI
        showSigningSuccess(req.type);
        
    } catch (error) {
        console.error('Signing failed:', error);
        
        // Report error
        await apiCall('report_signing_result', {
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
        const messageType = type === 'personal_sign' ? 'Message' : 'Typed data';
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