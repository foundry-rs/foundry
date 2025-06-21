// API wrapper for backend communication
async function apiCall(endpoint, options = {}) {
    try {
        const response = await fetch(`/${endpoint}`, {
            ...options,
            headers: {
                'Content-Type': 'application/json',
                ...options.headers
            }
        });
        
        if (!response.ok) {
            throw new Error(`HTTP error! status: ${response.status}`);
        }
        
        return await response.json();
    } catch (error) {
        console.error(`API call failed: ${endpoint}`, error);
        return null;
    }
}

// UI update helpers
function updateStatus(status) {
    const element = document.getElementById('connection-status');
    if (element) {
        element.textContent = status;
        element.className = 'status-value ' + (status === 'Connected' ? 'connected' : '');
    }
}

function updateAccount(address) {
    const element = document.getElementById('account-address');
    if (element) {
        element.textContent = address || 'None';
    }
}

function updateNetwork(networkName) {
    const element = document.getElementById('network-name');
    if (element) {
        element.textContent = networkName || 'Unknown';
    }
}

function showError(message) {
    const errorContainer = document.getElementById('error-container');
    const errorMessage = document.getElementById('error-message');
    
    if (errorContainer && errorMessage) {
        errorMessage.textContent = message;
        errorContainer.style.display = 'block';
        
        setTimeout(() => {
            errorContainer.style.display = 'none';
        }, 5000);
    }
}

function hideError() {
    const errorContainer = document.getElementById('error-container');
    if (errorContainer) {
        errorContainer.style.display = 'none';
    }
}

// Format wei to ETH for display
function formatWei(weiString) {
    try {
        const wei = BigInt(weiString);
        const eth = Number(wei) / 1e18;
        return `${weiString} wei (${eth.toFixed(6)} ETH)`;
    } catch {
        return `${weiString} wei`;
    }
}

// Shorten address for display
function shortenAddress(address) {
    if (!address || address.length < 42) return address;
    return `${address.slice(0, 6)}...${address.slice(-4)}`;
}