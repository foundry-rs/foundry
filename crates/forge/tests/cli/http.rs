//! Tests for the `httpRequest` and `httpGet` cheatcodes.

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect},
    routing::{get, post},
};
use foundry_test_utils::{forgetest_init, str, util::OutputExt};
use tokio::net::TcpListener;

/// Serves a JSON price, an echo of POST requests, a 404, and redirects inside and outside the
/// allowlist.
async fn server() -> String {
    let app = Router::new()
        .route(
            "/api/price",
            get(|| async { ([(header::CONTENT_TYPE, "application/json")], r#"{"price":42}"#) }),
        )
        .route(
            "/api/echo",
            post(|headers: HeaderMap, body: String| async move {
                let authorized = headers
                    .get(header::AUTHORIZATION)
                    .is_some_and(|v| v.as_bytes() == b"Bearer secret-token");
                (StatusCode::CREATED, format!("authorized={authorized}|{body}"))
            }),
        )
        .route("/api/missing", get(|| async { StatusCode::NOT_FOUND.into_response() }))
        .route("/api/redirect", get(|| async { Redirect::temporary("http://localhost:1/") }))
        .route("/api/hop", get(|| async { Redirect::temporary("/api/price") }))
        // 11 MiB in chunks, with no Content-Length.
        .route(
            "/api/large",
            get(|| async {
                let chunks = (0..11).map(|_| Ok::<_, std::io::Error>(vec![b'a'; 1024 * 1024]));
                Body::from_stream(futures::stream::iter(chunks))
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    endpoint
}

const TEST: &str = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";

interface Http {
    struct HttpRequest {
        string method;
        string url;
        string[] headers;
        bytes body;
    }

    struct HttpResponse {
        uint256 status;
        string[] headers;
        bytes body;
    }

    function httpRequest(HttpRequest calldata request) external returns (HttpResponse memory);
    function httpGet(string calldata url) external returns (HttpResponse memory);
}

contract HttpTest is Test {
    Http constant http = Http(address(uint160(uint256(keccak256("hevm cheat code")))));
    string endpoint;

    function setUp() public {
        endpoint = vm.envString("HTTP_ENDPOINT");
    }

    function test_get() public {
        Http.HttpResponse memory res = http.httpGet(string.concat(endpoint, "/api/price"));
        assertEq(res.status, 200);
        assertEq(vm.parseJsonUint(string(res.body), ".price"), 42);
        bool hasContentType;
        for (uint256 i; i < res.headers.length; i++) {
            if (keccak256(bytes(res.headers[i])) == keccak256("content-type: application/json")) {
                hasContentType = true;
            }
        }
        assertTrue(hasContentType);
    }

    function test_post() public {
        string[] memory headers = new string[](1);
        headers[0] = "Authorization: Bearer secret-token";
        Http.HttpResponse memory res = http.httpRequest(
            Http.HttpRequest("post", string.concat(endpoint, "/api/echo"), headers, bytes("hello"))
        );
        assertEq(res.status, 201);
        assertEq(string(res.body), "authorized=true|hello");
    }

    function test_error_status_returns() public {
        assertEq(http.httpGet(string.concat(endpoint, "/api/missing")).status, 404);
    }

    function test_redirect_within_allowlist_is_followed() public {
        Http.HttpResponse memory res = http.httpGet(string.concat(endpoint, "/api/hop"));
        assertEq(res.status, 200);
        assertEq(vm.parseJsonUint(string(res.body), ".price"), 42);
    }

    function test_redirect_outside_allowlist_is_not_followed() public {
        assertEq(http.httpGet(string.concat(endpoint, "/api/redirect")).status, 307);
    }

    function test_large_body() public {
        http.httpGet(string.concat(endpoint, "/api/large"));
    }

    function test_host_header() public {
        string[] memory headers = new string[](1);
        headers[0] = "Host: admin.internal";
        http.httpRequest(Http.HttpRequest("GET", string.concat(endpoint, "/api/price"), headers, ""));
    }

    function test_not_allowed() public {
        http.httpGet(string.concat(endpoint, "/private?key=hidden"));
    }
}
"#;

#[forgetest_init]
async fn http_cheatcodes(prj: _, cmd: _) {
    let endpoint = server().await;
    prj.update_config(|config| config.http_allowlist = vec![format!("{endpoint}/api/")]);
    prj.add_test("Http.t.sol", TEST);
    cmd.env("HTTP_ENDPOINT", &endpoint);

    cmd.args([
        "test",
        "--mc",
        "HttpTest",
        "--nmt",
        "test_not_allowed|test_large_body|test_host_header",
    ])
    .assert_success();

    // The path and query are left out of the error, as they may hold credentials.
    cmd.forge_fuse().env("HTTP_ENDPOINT", &endpoint);
    cmd.args(["test", "--mt", "test_not_allowed"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: vm.httpGet: http://127.0.0.1:[..] is not allowed; add the URL to `http_allowlist` in foundry.toml] test_not_allowed() ([GAS])
...
"#]]);

    cmd.forge_fuse().env("HTTP_ENDPOINT", &endpoint);
    cmd.args(["test", "--mt", "test_large_body|test_host_header"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: vm.httpRequest: the `Host` header cannot be set] test_host_header() ([GAS])
[FAIL: vm.httpGet: response from http://127.0.0.1:[..] is larger than 10485760 bytes] test_large_body() ([GAS])
...
"#]]);

    // Traces show the method and origin only, and hide the response below -vvvvv.
    cmd.forge_fuse().env("HTTP_ENDPOINT", &endpoint);
    let output = cmd
        .args(["test", "--mt", "test_post", "-vvvv"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(output.contains("httpRequest(POST http://127.0.0.1:"), "{output}");
    assert!(output.contains("<http response>"), "{output}");
    assert!(!output.contains("secret-token"), "{output}");
}

#[forgetest_init]
async fn http_cheatcodes_disabled_by_default(prj: _, cmd: _) {
    let endpoint = server().await;
    prj.add_test("Http.t.sol", TEST);
    cmd.env("HTTP_ENDPOINT", &endpoint);
    cmd.args(["test", "--mt", "test_get"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: vm.httpGet: http://127.0.0.1:[..] is not allowed; add the URL to `http_allowlist` in foundry.toml] test_get() ([GAS])
...
"#]]);
}
