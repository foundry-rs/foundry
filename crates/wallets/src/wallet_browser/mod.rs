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

    use alloy_primitives::{Address, Bytes, TxHash, TxKind, U256, address};
    use alloy_rpc_types::TransactionRequest;
    use axum::http::{HeaderMap, HeaderValue};
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    use crate::wallet_browser::{
        error::BrowserWalletError,
        server::BrowserWalletServer,
        types::{
            BrowserApiResponse, BrowserSignRequest, BrowserSignResponse, BrowserTransactionRequest,
            BrowserTransactionResponse, Connection, SignRequest, SignType,
        },
    };

    const ALICE: Address = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    const BOB: Address = address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8");

    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);
    const DEFAULT_DEVELOPMENT: bool = false;

    #[tokio::test]
    async fn test_setup_server() {
        let mut server = create_server();
        let client = client_with_token(&server);

        // Check initial state
        assert!(!server.is_connected().await);
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
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();

        // Check that the transaction request queue is empty
        check_transaction_request_queue_empty(&client, &server).await;

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Check connection state
        let Connection { address, chain_id } =
            server.get_connection().await.expect("expected an active wallet connection");
        assert_eq!(address, ALICE);
        assert_eq!(chain_id, 1);

        // Disconnect wallet
        disconnect_wallet(&client, &server).await;

        // Check disconnected state
        assert!(!server.is_connected().await);

        // Connect Bob's wallet
        connect_wallet(&client, &server, Connection::new(BOB, 42)).await;

        // Check connection state
        let Connection { address, chain_id } =
            server.get_connection().await.expect("expected an active wallet connection");
        assert_eq!(address, BOB);
        assert_eq!(chain_id, 42);

        // Stop server
        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_switch_wallet() {
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();

        // Connect Alice, assert connected
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;
        let Connection { address, chain_id } =
            server.get_connection().await.expect("expected an active wallet connection");
        assert_eq!(address, ALICE);
        assert_eq!(chain_id, 1);

        // Connect Bob, assert switched
        connect_wallet(&client, &server, Connection::new(BOB, 42)).await;
        let Connection { address, chain_id } =
            server.get_connection().await.expect("expected an active wallet connection");
        assert_eq!(address, BOB);
        assert_eq!(chain_id, 42);

        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_transaction_response_both_hash_and_error_rejected() {
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Enqueue a tx
        let (tx_request_id, tx_request) = create_browser_transaction_request();
        let _handle = wait_for_transaction_signing(&server, tx_request).await;
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Wallet posts both hash and error -> should be rejected
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&BrowserTransactionResponse {
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
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        let (tx_request_id, tx_request) = create_browser_transaction_request();
        let _handle = wait_for_transaction_signing(&server, tx_request).await;
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Neither hash nor error -> rejected
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&BrowserTransactionResponse { id: tx_request_id, hash: None, error: None })
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
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        let (tx_request_id, tx_request) = create_browser_transaction_request();
        let _handle = wait_for_transaction_signing(&server, tx_request).await;
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Zero hash -> rejected
        let zero = TxHash::new([0u8; 32]);
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&BrowserTransactionResponse { id: tx_request_id, hash: Some(zero), error: None })
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
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        let (tx_request_id, tx_request) = create_browser_transaction_request();
        let handle = wait_for_transaction_signing(&server, tx_request).await;
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Simulate the wallet accepting and signing the tx
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&BrowserTransactionResponse {
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
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create a random transaction response without a matching request
        let tx_request_id = Uuid::new_v4();

        // Simulate the wallet sending a response for an unknown request
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&BrowserTransactionResponse {
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
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
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
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create a browser transaction request
        let (tx_request_id, tx_request) = create_browser_transaction_request();

        // Spawn the transaction signing flow in the background
        let handle = wait_for_transaction_signing(&server, tx_request).await;

        // Check transaction request
        check_transaction_request_content(&client, &server, tx_request_id).await;

        // Simulate the wallet rejecting the tx
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&BrowserTransactionResponse {
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
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create multiple browser transaction requests
        let (tx_request_id1, tx_request1) = create_browser_transaction_request();
        let (tx_request_id2, tx_request2) = create_different_browser_transaction_request();

        // Spawn signing flows for both transactions concurrently
        let handle1 = wait_for_transaction_signing(&server, tx_request1.clone()).await;
        let handle2 = wait_for_transaction_signing(&server, tx_request2.clone()).await;

        // Check first transaction request
        {
            let resp = client
                .get(format!("http://localhost:{}/api/transaction/request", server.port()))
                .send()
                .await
                .unwrap();

            let BrowserApiResponse::Ok(pending_tx) =
                resp.json::<BrowserApiResponse<BrowserTransactionRequest>>().await.unwrap()
            else {
                panic!("expected BrowserApiResponse::Ok with a pending transaction");
            };

            assert_eq!(
                pending_tx.id, tx_request_id1,
                "expected the first transaction to be at the front of the queue"
            );
            assert_eq!(pending_tx.request.from, tx_request1.request.from);
            assert_eq!(pending_tx.request.to, tx_request1.request.to);
            assert_eq!(pending_tx.request.value, tx_request1.request.value);
        }

        // Simulate the wallet accepting and signing the first transaction
        let resp1 = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&BrowserTransactionResponse {
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
            let resp = client
                .get(format!("http://localhost:{}/api/transaction/request", server.port()))
                .send()
                .await
                .unwrap();

            let BrowserApiResponse::Ok(pending_tx) =
                resp.json::<BrowserApiResponse<BrowserTransactionRequest>>().await.unwrap()
            else {
                panic!("expected BrowserApiResponse::Ok with a pending transaction");
            };

            assert_eq!(
                pending_tx.id, tx_request_id2,
                "expected the second transaction to be pending after the first one completed"
            );
            assert_eq!(pending_tx.request.from, tx_request2.request.from);
            assert_eq!(pending_tx.request.to, tx_request2.request.to);
            assert_eq!(pending_tx.request.value, tx_request2.request.value);
        }

        // Simulate the wallet rejecting the second transaction
        let resp2 = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&BrowserTransactionResponse {
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

    #[tokio::test]
    async fn test_send_sign_response_both_signature_and_error_rejected() {
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        let (sign_request_id, sign_request) = create_browser_sign_request();
        let _handle = wait_for_message_signing(&server, sign_request).await;
        check_sign_request_content(&client, &server, sign_request_id).await;

        // Both signature and error -> should be rejected
        let resp = client
            .post(format!("http://localhost:{}/api/signing/response", server.port()))
            .json(&BrowserSignResponse {
                id: sign_request_id,
                signature: Some(Bytes::from("Hello World")),
                error: Some("Should not have both".into()),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let api: BrowserApiResponse<()> = resp.json().await.unwrap();
        match api {
            BrowserApiResponse::Error { message } => {
                assert_eq!(message, "Only one of signature or error can be provided");
            }
            _ => panic!("expected error response"),
        }
    }

    #[tokio::test]
    async fn test_send_sign_response_neither_hash_nor_error_rejected() {
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        let (sign_request_id, sign_request) = create_browser_sign_request();
        let _handle = wait_for_message_signing(&server, sign_request).await;
        check_sign_request_content(&client, &server, sign_request_id).await;

        // Neither signature nor error -> rejected
        let resp = client
            .post(format!("http://localhost:{}/api/signing/response", server.port()))
            .json(&BrowserSignResponse { id: sign_request_id, signature: None, error: None })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let api: BrowserApiResponse<()> = resp.json().await.unwrap();
        match api {
            BrowserApiResponse::Error { message } => {
                assert_eq!(message, "Either signature or error must be provided");
            }
            _ => panic!("expected error response"),
        }
    }

    #[tokio::test]
    async fn test_send_sign_client_accept() {
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        let (sign_request_id, sign_request) = create_browser_sign_request();
        let handle = wait_for_message_signing(&server, sign_request).await;
        check_sign_request_content(&client, &server, sign_request_id).await;

        // Simulate the wallet accepting and signing the message
        let resp = client
            .post(format!("http://localhost:{}/api/signing/response", server.port()))
            .json(&BrowserSignResponse {
                id: sign_request_id,
                signature: Some(Bytes::from("FakeSignature")),
                error: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // The join handle should now return the signature
        let res = handle.await.expect("task panicked");
        match res {
            Ok(signature) => {
                assert_eq!(signature, Bytes::from("FakeSignature"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_send_sign_client_not_requested() {
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create a random signing response without a matching request
        let sign_request_id = Uuid::new_v4();

        // Simulate the wallet sending a response for an unknown request
        let resp = client
            .post(format!("http://localhost:{}/api/signing/response", server.port()))
            .json(&BrowserSignResponse {
                id: sign_request_id,
                signature: Some(Bytes::from("FakeSignature")),
                error: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // Assert that no signing response without a matching request is accepted
        let api: BrowserApiResponse<()> = resp.json().await.unwrap();
        match api {
            BrowserApiResponse::Error { message } => {
                assert_eq!(message, "Unknown signing request id");
            }
            _ => panic!("expected error response"),
        }
    }

    #[tokio::test]
    async fn test_send_sign_invalid_response_format() {
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Simulate the wallet sending a response with an invalid UUID
        let resp = client
            .post(format!("http://localhost:{}/api/signing/response", server.port()))
            .body(
                r#"{
                "id": "invalid-uuid",
                "signature": "invalid-signature",
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
    async fn test_send_sign_client_reject() {
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        let (sign_request_id, sign_request) = create_browser_sign_request();
        let handle = wait_for_message_signing(&server, sign_request).await;
        check_sign_request_content(&client, &server, sign_request_id).await;

        // Simulate the wallet rejecting the signing request
        let resp = client
            .post(format!("http://localhost:{}/api/signing/response", server.port()))
            .json(&BrowserSignResponse {
                id: sign_request_id,
                signature: None,
                error: Some("User rejected the signing request".into()),
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
                assert_eq!(operation, "Signing");
                assert_eq!(reason, "User rejected the signing request");
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_send_multiple_sign_requests() {
        let mut server = create_server();
        let client = client_with_token(&server);
        server.start().await.unwrap();
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create multiple browser sign requests
        let (sign_request_id1, sign_request1) = create_browser_sign_request();
        let (sign_request_id2, sign_request2) = create_different_browser_sign_request();

        // Spawn signing flows for both sign requests concurrently
        let handle1 = wait_for_message_signing(&server, sign_request1.clone()).await;
        let handle2 = wait_for_message_signing(&server, sign_request2.clone()).await;

        // Check first sign request
        {
            let resp = client
                .get(format!("http://localhost:{}/api/signing/request", server.port()))
                .send()
                .await
                .unwrap();

            let BrowserApiResponse::Ok(pending_sign) =
                resp.json::<BrowserApiResponse<BrowserSignRequest>>().await.unwrap()
            else {
                panic!("expected BrowserApiResponse::Ok with a pending sign request");
            };

            assert_eq!(pending_sign.id, sign_request_id1);
            assert_eq!(pending_sign.sign_type, sign_request1.sign_type);
            assert_eq!(pending_sign.request.address, sign_request1.request.address);
            assert_eq!(pending_sign.request.message, sign_request1.request.message);
        }

        // Simulate the wallet accepting and signing the first sign request
        let resp1 = client
            .post(format!("http://localhost:{}/api/signing/response", server.port()))
            .json(&BrowserSignResponse {
                id: sign_request_id1,
                signature: Some(Bytes::from("Signature1")),
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
            Ok(signature) => assert_eq!(signature, Bytes::from("Signature1")),
            other => panic!("expected success, got {other:?}"),
        }

        // Check second sign request
        {
            let resp = client
                .get(format!("http://localhost:{}/api/signing/request", server.port()))
                .send()
                .await
                .unwrap();

            let BrowserApiResponse::Ok(pending_sign) =
                resp.json::<BrowserApiResponse<BrowserSignRequest>>().await.unwrap()
            else {
                panic!("expected BrowserApiResponse::Ok with a pending sign request");
            };

            assert_eq!(pending_sign.id, sign_request_id2,);
            assert_eq!(pending_sign.sign_type, sign_request2.sign_type);
            assert_eq!(pending_sign.request.address, sign_request2.request.address);
            assert_eq!(pending_sign.request.message, sign_request2.request.message);
        }

        // Simulate the wallet rejecting the second sign request
        let resp2 = client
            .post(format!("http://localhost:{}/api/signing/response", server.port()))
            .json(&BrowserSignResponse {
                id: sign_request_id2,
                signature: None,
                error: Some("User rejected the signing request".into()),
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
                assert_eq!(operation, "Signing");
                assert_eq!(reason, "User rejected the signing request");
            }
            other => panic!("expected BrowserWalletError::Rejected, got {other:?}"),
        }

        check_sign_request_queue_empty(&client, &server).await;

        server.stop().await.unwrap();
    }

    /// Helper to create a default browser wallet server.
    fn create_server() -> BrowserWalletServer {
        BrowserWalletServer::new(0, false, DEFAULT_TIMEOUT, DEFAULT_DEVELOPMENT)
    }

    /// Helper to create a reqwest client with the session token header.
    fn client_with_token(server: &BrowserWalletServer) -> reqwest::Client {
        let mut headers = HeaderMap::new();
        headers.insert("X-Session-Token", HeaderValue::from_str(server.session_token()).unwrap());
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

    /// Spawn the transaction signing flow in the background and return the join handle.
    async fn wait_for_transaction_signing(
        server: &BrowserWalletServer,
        tx_request: BrowserTransactionRequest,
    ) -> JoinHandle<Result<TxHash, BrowserWalletError>> {
        // Spawn the signing flow in the background
        let browser_server = server.clone();
        let join_handle =
            tokio::spawn(async move { browser_server.request_transaction(tx_request).await });
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        join_handle
    }

    /// Spawn the message signing flow in the background and return the join handle.
    async fn wait_for_message_signing(
        server: &BrowserWalletServer,
        sign_request: BrowserSignRequest,
    ) -> JoinHandle<Result<Bytes, BrowserWalletError>> {
        // Spawn the signing flow in the background
        let browser_server = server.clone();
        let join_handle =
            tokio::spawn(async move { browser_server.request_signing(sign_request).await });
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        join_handle
    }

    /// Create a simple browser transaction request.
    fn create_browser_transaction_request() -> (Uuid, BrowserTransactionRequest) {
        let id = Uuid::new_v4();
        let tx = BrowserTransactionRequest {
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
    fn create_different_browser_transaction_request() -> (Uuid, BrowserTransactionRequest) {
        let id = Uuid::new_v4();
        let tx = BrowserTransactionRequest {
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

    /// Create a simple browser sign request.
    fn create_browser_sign_request() -> (Uuid, BrowserSignRequest) {
        let id = Uuid::new_v4();
        let req = BrowserSignRequest {
            id,
            sign_type: SignType::PersonalSign,
            request: SignRequest { message: "Hello, world!".into(), address: ALICE },
        };
        (id, req)
    }

    /// Create a different browser sign request (from the first one).
    fn create_different_browser_sign_request() -> (Uuid, BrowserSignRequest) {
        let id = Uuid::new_v4();
        let req = BrowserSignRequest {
            id,
            sign_type: SignType::SignTypedDataV4,
            request: SignRequest { message: "Different message".into(), address: BOB },
        };
        (id, req)
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
            resp.json::<BrowserApiResponse<BrowserTransactionRequest>>().await.unwrap()
        else {
            panic!("expected BrowserApiResponse::Error (no pending transaction), but got Ok");
        };

        assert_eq!(message, "No pending transaction request");
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
            resp.json::<BrowserApiResponse<BrowserTransactionRequest>>().await.unwrap()
        else {
            panic!("expected BrowserApiResponse::Ok with a pending transaction");
        };

        assert_eq!(pending_tx.id, tx_request_id);
        assert_eq!(pending_tx.request.from, Some(ALICE));
        assert_eq!(pending_tx.request.to, Some(TxKind::Call(BOB)));
        assert_eq!(pending_tx.request.value, Some(U256::from(1000)));
    }

    /// Check that the sign request queue is empty, if not panic.
    async fn check_sign_request_queue_empty(
        client: &reqwest::Client,
        server: &BrowserWalletServer,
    ) {
        let resp = client
            .get(format!("http://localhost:{}/api/signing/request", server.port()))
            .send()
            .await
            .unwrap();

        let BrowserApiResponse::Error { message } =
            resp.json::<BrowserApiResponse<BrowserSignRequest>>().await.unwrap()
        else {
            panic!("expected BrowserApiResponse::Error (no pending signing request), but got Ok");
        };

        assert_eq!(message, "No pending signing request");
    }

    /// Check that the sign request matches the expected request ID and fields.
    async fn check_sign_request_content(
        client: &reqwest::Client,
        server: &BrowserWalletServer,
        sign_request_id: Uuid,
    ) {
        let resp = client
            .get(format!("http://localhost:{}/api/signing/request", server.port()))
            .send()
            .await
            .unwrap();

        let BrowserApiResponse::Ok(pending_req) =
            resp.json::<BrowserApiResponse<BrowserSignRequest>>().await.unwrap()
        else {
            panic!("expected BrowserApiResponse::Ok with a pending signing request");
        };

        assert_eq!(pending_req.id, sign_request_id);
        assert_eq!(pending_req.sign_type, SignType::PersonalSign);
        assert_eq!(pending_req.request.address, ALICE);
        assert_eq!(pending_req.request.message, "Hello, world!");
    }
}
