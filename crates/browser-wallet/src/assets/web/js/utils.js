// Storage keys
const STORAGE_KEYS = {
    LAST_CONNECTED_ACCOUNT: 'foundry.wallet.lastAccount',
    LAST_CHAIN_ID: 'foundry.wallet.lastChainId',
    CONNECTION_VERSION: 'foundry.wallet.version',
    WALLET_CONNECTED: 'foundry.wallet.connected'
};

// Current version for storage migration
const STORAGE_VERSION = '1.0.0';

// Connection state persistence
const connectionStorage = {
    saveConnection(address, chainId) {
        try {
            localStorage.setItem(STORAGE_KEYS.LAST_CONNECTED_ACCOUNT, address);
            localStorage.setItem(STORAGE_KEYS.LAST_CHAIN_ID, chainId.toString());
            localStorage.setItem(STORAGE_KEYS.CONNECTION_VERSION, STORAGE_VERSION);
            localStorage.setItem(STORAGE_KEYS.WALLET_CONNECTED, 'true');
        } catch (e) {
            console.warn('Failed to save connection state:', e);
        }
    },

    getLastConnection() {
        try {
            const version = localStorage.getItem(STORAGE_KEYS.CONNECTION_VERSION);
            // Clear old versions
            if (version && version !== STORAGE_VERSION) {
                this.clearConnection();
                return null;
            }

            const wasConnected = localStorage.getItem(STORAGE_KEYS.WALLET_CONNECTED) === 'true';
            if (!wasConnected) return null;

            return {
                address: localStorage.getItem(STORAGE_KEYS.LAST_CONNECTED_ACCOUNT),
                chainId: parseInt(localStorage.getItem(STORAGE_KEYS.LAST_CHAIN_ID) || '0')
            };
        } catch (e) {
            console.warn('Failed to get connection state:', e);
            return null;
        }
    },

    clearConnection() {
        try {
            localStorage.removeItem(STORAGE_KEYS.LAST_CONNECTED_ACCOUNT);
            localStorage.removeItem(STORAGE_KEYS.LAST_CHAIN_ID);
            localStorage.removeItem(STORAGE_KEYS.WALLET_CONNECTED);
        } catch (e) {
            console.warn('Failed to clear connection state:', e);
        }
    }
};

// API wrapper for backend communication with timeout
async function apiCall(endpoint, options = {}, timeout = 30000) {
    // Create abort controller for timeout
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), timeout);
    
    try {
        const response = await fetch(`/${endpoint}`, {
            ...options,
            headers: {
                'Content-Type': 'application/json',
                ...options.headers
            },
            signal: controller.signal,
            credentials: 'same-origin' // Important for CSRF protection
        });
        
        clearTimeout(timeoutId);
        
        if (!response.ok) {
            throw new Error(`HTTP error! status: ${response.status}`);
        }
        
        return await response.json();
    } catch (error) {
        clearTimeout(timeoutId);
        
        if (error.name === 'AbortError') {
            console.error(`API call timed out after ${timeout}ms: ${endpoint}`);
            showError('Request timed out. Please try again.');
        } else {
            console.error(`API call failed: ${endpoint}`, error);
        }
        
        return null;
    }
}

// UI update helpers
function updateStatus(status) {
    const element = document.getElementById('connection-status');
    if (element) {
        element.textContent = status;
        let className = 'status-value';
        if (status === 'Connected') {
            className += ' connected';
        } else if (status === 'Initializing...') {
            className += ' initializing';
        }
        element.className = className;
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

// Provider detection with exponential backoff
async function detectProvider(maxRetries = 10, initialDelay = 100) {
    let retries = 0;
    let delay = initialDelay;
    
    return new Promise((resolve) => {
        const checkProvider = () => {
            if (window.ethereum) {
                console.log('Provider detected after', retries, 'attempts');
                resolve(true);
                return;
            }
            
            retries++;
            if (retries >= maxRetries) {
                console.warn('Provider not detected after', maxRetries, 'attempts');
                resolve(false);
                return;
            }
            
            // Exponential backoff with max delay of 5 seconds
            delay = Math.min(delay * 1.5, 5000);
            setTimeout(checkProvider, delay);
        };
        
        checkProvider();
    });
}

// Connection state machine
const ConnectionState = {
    INITIALIZING: 'initializing',
    PROVIDER_NOT_FOUND: 'provider_not_found',
    READY: 'ready',
    CONNECTING: 'connecting',
    CONNECTED: 'connected',
    DISCONNECTED: 'disconnected',
    ERROR: 'error'
};

// Global connection state
let connectionState = ConnectionState.INITIALIZING;
let connectionStateListeners = [];

function setConnectionState(newState) {
    const oldState = connectionState;
    connectionState = newState;
    console.log('Connection state:', oldState, '->', newState);
    
    // Notify listeners
    connectionStateListeners.forEach(listener => {
        listener(newState, oldState);
    });
}

function onConnectionStateChange(listener) {
    connectionStateListeners.push(listener);
    // Return unsubscribe function
    return () => {
        connectionStateListeners = connectionStateListeners.filter(l => l !== listener);
    };
}

// Check if we should attempt auto-reconnection
async function shouldAutoReconnect() {
    const lastConnection = connectionStorage.getLastConnection();
    if (!lastConnection || !lastConnection.address) return false;
    
    // Check if the account is still available
    try {
        const accounts = await window.ethereum.request({ method: 'eth_accounts' });
        return accounts.includes(lastConnection.address);
    } catch (error) {
        console.error('Failed to check accounts for auto-reconnect:', error);
        return false;
    }
}