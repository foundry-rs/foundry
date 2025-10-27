pub mod error;
pub mod server;
pub mod signer;
pub mod state;

mod app;
mod handlers;
mod queue;
mod router;
mod types;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use alloy_primitives::{Address, TxHash, TxKind, U256, address};
    use alloy_rpc_types::TransactionRequest;
    use axum::http::{HeaderMap, HeaderValue};
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    use crate::wallet_browser::{
        error::BrowserWalletError,
        server::BrowserWalletServer,
        types::{BrowserApiResponse, BrowserTransaction, Connection, TransactionResponse},
    };

    const ALICE: Address = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    const BOB: Address = address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8");

    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

    #[tokio::test]
    async fn test_setup_server() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);

        // Check initial state
        assert!(!server.is_connected());
        assert!(!server.open_browser());
        assert!(server.timeout() == DEFAULT_TIMEOUT);

        // Start server
        server.start().await.unwrap();

        // Check that the transaction request queue is empty
        check_transaction_request_queue_empty(&client, &server).await;

        // Stop server
        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_connect_disconnect_wallet() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();

        // Check that the transaction request queue is empty
        check_transaction_request_queue_empty(&client, &server).await;

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Check connection state
        let Connection(address, chain_id) =
            server.get_connection().expect("expected an active wallet connection");
        assert_eq!(address, ALICE);
        assert_eq!(chain_id, 1);

        // Disconnect wallet
        disconnect_wallet(&client, &server).await;

        // Check disconnected state
        assert!(!server.is_connected());

        // Connect Bob's wallet
        connect_wallet(&client, &server, Connection::new(BOB, 42)).await;

        // Check connection state
        let Connection(address, chain_id) =
            server.get_connection().expect("expected an active wallet connection");
        assert_eq!(address, BOB);
        assert_eq!(chain_id, 42);

        // Stop server
        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_switch_wallet() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();

        // Connect Alice, assert connected
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;
        let Connection(address, chain_id) =
            server.get_connection().expect("expected an active wallet connection");
        assert_eq!(address, ALICE);
        assert_eq!(chain_id, 1);

        // Connect Bob, assert switched
        connect_wallet(&client, &server, Connection::new(BOB, 42)).await;
        let Connection(address, chain_id) =
            server.get_connection().expect("expected an active wallet connection");
        assert_eq!(address, BOB);
        assert_eq!(chain_id, 42);

        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_transaction_response_both_hash_and_error_rejected() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Enqueue a tx
        let (tx_request_id, tx_request) = create_browser_transaction();
        let _handle = wait_for_signing(&server, tx_request).await;
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Wallet posts both hash and error -> should be rejected
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse {
                id: tx_request_id,
                hash: Some(TxHash::random()),
                error: Some("should not have both".into()),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let api: BrowserApiResponse<()> = resp.json().await.unwrap();
        match api {
            BrowserApiResponse::Error { message } => {
                assert_eq!(message, "Only one of hash or error can be provided");
            }
            _ => panic!("expected error response"),
        }
    }

    #[tokio::test]
    async fn test_transaction_response_neither_hash_nor_error_rejected() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        let (tx_request_id, tx_request) = create_browser_transaction();
        let _handle = wait_for_signing(&server, tx_request).await;
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Neither hash nor error -> rejected
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse { id: tx_request_id, hash: None, error: None })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let api: BrowserApiResponse<()> = resp.json().await.unwrap();
        match api {
            BrowserApiResponse::Error { message } => {
                assert_eq!(message, "Either hash or error must be provided");
            }
            _ => panic!("expected error response"),
        }
    }

    #[tokio::test]
    async fn test_transaction_response_zero_hash_rejected() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        let (tx_request_id, tx_request) = create_browser_transaction();
        let _handle = wait_for_signing(&server, tx_request).await;
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Zero hash -> rejected
        let zero = TxHash::new([0u8; 32]);
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse { id: tx_request_id, hash: Some(zero), error: None })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let api: BrowserApiResponse<()> = resp.json().await.unwrap();
        match api {
            BrowserApiResponse::Error { message } => {
                // Message text per your handler; adjust if you use a different string.
                assert!(
                    message.contains("Invalid") || message.contains("Malformed"),
                    "unexpected message: {message}"
                );
            }
            _ => panic!("expected error response"),
        }
    }

    #[tokio::test]
    async fn test_send_transaction_client_accept() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create a browser transaction request
        let (tx_request_id, tx_request) = create_browser_transaction();

        // Spawn the signing flow in the background
        let handle = wait_for_signing(&server, tx_request).await;

        // Check transaction request
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Simulate the wallet accepting and signing the tx
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse {
                id: tx_request_id,
                hash: Some(TxHash::random()),
                error: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // The join handle should now return the tx hash
        let res = handle.await.expect("task panicked");
        match res {
            Ok(hash) => {
                assert!(hash != TxHash::new([0; 32]));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_send_transaction_client_not_requested() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create a random transaction response without a matching request
        let tx_request_id = Uuid::new_v4();

        // Simulate the wallet sending a response for an unknown request
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse {
                id: tx_request_id,
                hash: Some(TxHash::random()),
                error: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // Assert that no transaction without a matching request is accepted
        let api: BrowserApiResponse<()> = resp.json().await.unwrap();
        match api {
            BrowserApiResponse::Error { message } => {
                assert_eq!(message, "Unknown transaction id");
            }
            _ => panic!("expected error response"),
        }
    }

    #[tokio::test]
    async fn test_send_transaction_invalid_response_format() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Simulate the wallet sending a response with an invalid UUID
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .body(
                r#"{
                "id": "invalid-uuid",
                "hash": "invalid-hash",
                "error": null
            }"#,
            )
            .header("Content-Type", "application/json")
            .send()
            .await
            .unwrap();

        // The server should respond with a 422 Unprocessable Entity status
        assert_eq!(resp.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_send_transaction_client_reject() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection(ALICE, 1)).await;

        // Create a browser transaction request
        let (tx_request_id, tx_request) = create_browser_transaction();

        // Spawn the signing flow in the background
        let handle = wait_for_signing(&server, tx_request).await;

        // Check transaction request
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Simulate the wallet rejecting the tx
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse {
                id: tx_request_id,
                hash: None,
                error: Some("User rejected the transaction".into()),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // The join handle should now return a rejection error
        let res = handle.await.expect("task panicked");
        match res {
            Err(BrowserWalletError::Rejected { operation, reason }) => {
                assert_eq!(operation, "Transaction");
                assert_eq!(reason, "User rejected the transaction");
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_send_multiple_transaction_requests() {
        let mut server = BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT);
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create multiple browser transaction requests
        let (tx_request_id1, tx_request1) = create_browser_transaction();
        let (tx_request_id2, tx_request2) = create_different_browser_transaction();

        // Spawn signing flows for both transactions concurrently
        let handle1 = wait_for_signing(&server, tx_request1).await;
        let handle2 = wait_for_signing(&server, tx_request2).await;

        // Check first transaction request
        {
            let url = format!("http://localhost:{}/api/transaction/request", server.port());
            let resp = reqwest::get(&url).await.unwrap();

            let BrowserApiResponse::Ok(pending_tx) =
                resp.json::<BrowserApiResponse<BrowserTransaction>>().await.unwrap()
            else {
                panic!("expected BrowserApiResponse::Ok with a pending transaction");
            };

            assert_eq!(
                pending_tx.id, tx_request_id1,
                "expected the first transaction to be at the front of the queue"
            );
            assert_eq!(pending_tx.request.from, Some(ALICE));
            assert_eq!(pending_tx.request.to, Some(TxKind::Call(BOB)));
            assert_eq!(pending_tx.request.value, Some(U256::from(1000)));
        }

        // Simulate the wallet accepting and signing the first transaction
        let resp1 = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse {
                id: tx_request_id1,
                hash: Some(TxHash::random()),
                error: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(resp1.status(), reqwest::StatusCode::OK);

        let res1 = handle1.await.expect("first signing flow panicked");
        match res1 {
            Ok(hash) => assert!(!hash.is_zero(), "first tx hash should not be zero"),
            other => panic!("expected success, got {other:?}"),
        }

        // Check second transaction request
        {
            let url = format!("http://localhost:{}/api/transaction/request", server.port());
            let resp = reqwest::get(&url).await.unwrap();

            let BrowserApiResponse::Ok(pending_tx) =
                resp.json::<BrowserApiResponse<BrowserTransaction>>().await.unwrap()
            else {
                panic!("expected BrowserApiResponse::Ok with a pending transaction");
            };

            assert_eq!(
                pending_tx.id, tx_request_id2,
                "expected the second transaction to be pending after the first one completed"
            );
            assert_eq!(pending_tx.request.from, Some(BOB));
            assert_eq!(pending_tx.request.to, Some(TxKind::Call(ALICE)));
            assert_eq!(pending_tx.request.value, Some(U256::from(2000)));
        }

        // Simulate the wallet rejecting the second transaction
        let resp2 = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse {
                id: tx_request_id2,
                hash: None,
                error: Some("User rejected the transaction".into()),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(resp2.status(), reqwest::StatusCode::OK);

        let res2 = handle2.await.expect("second signing flow panicked");
        match res2 {
            Err(BrowserWalletError::Rejected { operation, reason }) => {
                assert_eq!(operation, "Transaction");
                assert_eq!(reason, "User rejected the transaction");
            }
            other => panic!("expected BrowserWalletError::Rejected, got {other:?}"),
        }

        check_transaction_request_queue_empty(&client, &server).await;

        server.stop().await.unwrap();
    }

    /// Helper to create a reqwest client with the session token header.
    fn client_with_token(server: &BrowserWalletServer) -> reqwest::Client {
        let mut headers = HeaderMap::new();
        headers.insert("X-Session-Token", HeaderValue::from_str(&server.session_token()).unwrap());
        reqwest::Client::builder().default_headers(headers).build().unwrap()
    }

    /// Helper to connect a wallet to the server.
    async fn connect_wallet(
        client: &reqwest::Client,
        server: &BrowserWalletServer,
        connection: Connection,
    ) {
        let resp = client
            .post(format!("http://localhost:{}/api/connection", server.port()))
            .json(&connection)
            .send();
        assert!(resp.await.is_ok());
    }

    /// Helper to disconnect a wallet from the server.
    async fn disconnect_wallet(client: &reqwest::Client, server: &BrowserWalletServer) {
        let resp = client
            .post(format!("http://localhost:{}/api/connection", server.port()))
            .json(&Option::<Connection>::None)
            .send();
        assert!(resp.await.is_ok());
    }

    /// Spawn the signing flow in the background and return the join handle.
    async fn wait_for_signing(
        server: &BrowserWalletServer,
        tx_request: BrowserTransaction,
    ) -> JoinHandle<Result<TxHash, BrowserWalletError>> {
        // Spawn the signing flow in the background
        let browser_server = server.clone();
        let join_handle =
            tokio::spawn(async move { browser_server.request_transaction(tx_request).await });
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        join_handle
    }

    /// Create a simple browser transaction request.
    fn create_browser_transaction() -> (Uuid, BrowserTransaction) {
        let id = Uuid::new_v4();
        let tx = BrowserTransaction {
            id,
            request: TransactionRequest {
                from: Some(ALICE),
                to: Some(TxKind::Call(BOB)),
                value: Some(U256::from(1000)),
                ..Default::default()
            },
        };
        (id, tx)
    }

    /// Create a different browser transaction request (from the first one).
    fn create_different_browser_transaction() -> (Uuid, BrowserTransaction) {
        let id = Uuid::new_v4();
        let tx = BrowserTransaction {
            id,
            request: TransactionRequest {
                from: Some(BOB),
                to: Some(TxKind::Call(ALICE)),
                value: Some(U256::from(2000)),
                ..Default::default()
            },
        };
        (id, tx)
    }

    /// Check that the transaction request queue is empty, if not panic.
    async fn check_transaction_request_queue_empty(
        client: &reqwest::Client,
        server: &BrowserWalletServer,
    ) {
        let resp = client
            .get(format!("http://localhost:{}/api/transaction/request", server.port()))
            .send()
            .await
            .unwrap();

        let BrowserApiResponse::Error { message } =
            resp.json::<BrowserApiResponse<BrowserTransaction>>().await.unwrap()
        else {
            panic!("expected BrowserApiResponse::Error (no pending transaction), but got Ok");
        };

        assert_eq!(message, "No pending transaction");
    }

    /// Check that the transaction request matches the expected request ID and fields.
    async fn check_transaction_request_content(
        client: &reqwest::Client,
        server: &BrowserWalletServer,
        tx_request_id: Uuid,
    ) {
        let resp = client
            .get(format!("http://localhost:{}/api/transaction/request", server.port()))
            .send()
            .await
            .unwrap();

        let BrowserApiResponse::Ok(pending_tx) =
            resp.json::<BrowserApiResponse<BrowserTransaction>>().await.unwrap()
        else {
            panic!("expected BrowserApiResponse::Ok with a pending transaction");
        };

        assert_eq!(pending_tx.id, tx_request_id);
        assert_eq!(pending_tx.request.from, Some(ALICE));
        assert_eq!(pending_tx.request.to, Some(TxKind::Call(BOB)));
        assert_eq!(pending_tx.request.value, Some(U256::from(1000)));
    }
}
