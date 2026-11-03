//! Tests for the `httpRequest` and `httpGet` cheatcodes.

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::{IntoResponse, Redirect},
    routing::{get, post},
};
use foundry_test_utils::{
    forgetest_init,
    snapbox::{self, IntoData},
    str,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::net::TcpListener;

/// Serves a JSON price, an echo of POST requests, a 404, and redirects inside and outside the
/// allowlist.
async fn server() -> (String, Arc<AtomicUsize>) {
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
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
        .route("/api/outside-hop", get(|| async { Redirect::temporary("/private") }))
        .route("/private", get(|| async { "not allowed" }))
        // 11 MiB in chunks, with no Content-Length.
        .route(
            "/api/large",
            get(|| async {
                let chunks = (0..11).map(|_| Ok::<_, std::io::Error>(vec![b'a'; 1024 * 1024]));
                Body::from_stream(futures::stream::iter(chunks))
            }),
        )
        .layer(middleware::map_request(move |request: Request| {
            count.fetch_add(1, Ordering::SeqCst);
            async move { request }
        }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (endpoint, requests)
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
    function _expectCheatcodeRevert(bytes calldata revertData) external;
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

    function test_cross_origin_redirect_is_not_followed() public {
        assertEq(http.httpGet(string.concat(endpoint, "/api/redirect")).status, 307);
    }

    function test_redirect_outside_path_is_not_followed() public {
        assertEq(http.httpGet(string.concat(endpoint, "/api/outside-hop")).status, 307);
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

    function test_encoded_paths() public {
        string[5] memory paths = [
            "/api/..%2fprivate",
            "/api/%2e%2e%2Fprivate",
            "/api/..%5cprivate",
            "/api/%252e%252e%252fprivate",
            "/api/..;/private"
        ];
        for (uint256 i; i < paths.length; i++) {
            http._expectCheatcodeRevert(bytes(string.concat(
                "vm.httpGet: ", endpoint,
                " is not allowed; add the URL to `http_allowlist` in foundry.toml"
            )));
            http.httpGet(string.concat(endpoint, paths[i]));
        }
    }
}
"#;

#[forgetest_init]
async fn http_cheatcodes(prj: _, cmd: _) {
    let (endpoint, _) = server().await;
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
    cmd.args(["test", "--mt", "test_post", "-vvvv"]).assert_success().stdout_eq(str![[r#"
...
    ├─ [0] VM::httpRequest(POST http://127.0.0.1:[..])
    │   └─ ← [Return] <http response>
...
"#]]);
}

#[forgetest_init]
async fn http_cheatcodes_disabled_by_default(prj: _, cmd: _) {
    let (endpoint, _) = server().await;
    prj.add_test("Http.t.sol", TEST);
    cmd.env("HTTP_ENDPOINT", &endpoint);
    cmd.args(["test", "--mt", "test_get"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: vm.httpGet: http://127.0.0.1:[..] is not allowed; add the URL to `http_allowlist` in foundry.toml] test_get() ([GAS])
...
"#]]);
}

#[forgetest_init]
async fn http_cheatcodes_reject_encoded_paths(prj: _, cmd: _) {
    let (endpoint, requests) = server().await;
    prj.update_config(|config| config.http_allowlist = vec![format!("{endpoint}/api/")]);
    prj.add_test("Http.t.sol", TEST);
    cmd.env("HTTP_ENDPOINT", &endpoint);
    cmd.args(["test", "--mt", "test_encoded_paths"]).assert_success();
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    cmd.forge_fuse().env("HTTP_ENDPOINT", &endpoint);
    cmd.args(["test", "--mt", "test_get"]).assert_success();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[forgetest_init]
async fn http_script_does_not_repeat_post_when_relinking(prj: _, cmd: _) {
    let (endpoint, requests) = server().await;
    prj.update_config(|config| config.http_allowlist = vec![format!("{endpoint}/api/")]);
    prj.add_script(
        "Http.s.sol",
        &format!(
            r#"{TEST}
library HttpLib {{
    function value() external pure returns (uint256) {{ return 42; }}
}}
contract UsesHttpLib {{
    uint256 public value = HttpLib.value();
}}
contract HttpSenderScript is HttpTest {{
    function run() external {{
        http.httpRequest(Http.HttpRequest(
            "POST", string.concat(endpoint, "/api/echo"), new string[](0), bytes("hello")
        ));
        vm.startBroadcast(uint256(1));
        new UsesHttpLib();
        vm.stopBroadcast();
    }}
}}
"#
        ),
    );
    cmd.env("HTTP_ENDPOINT", &endpoint);
    cmd.args(["script", "HttpSenderScript", "--skip-simulation"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Cannot rerun a script with HTTP enabled to relink libraries. Run again with --sender 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf to avoid repeating HTTP requests.

"#]]);
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    cmd.forge_fuse().env("HTTP_ENDPOINT", &endpoint);
    cmd.args([
        "script",
        "HttpSenderScript",
        "--skip-simulation",
        "--sender",
        "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
    ])
    .assert_success();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[forgetest_init]
async fn http_script_json_redacts_request_and_response(prj: _, cmd: _) {
    let (endpoint, _) = server().await;
    prj.update_config(|config| config.http_allowlist = vec![format!("{endpoint}/api/")]);
    prj.add_script(
        "HttpJson.s.sol",
        &format!(
            r#"{TEST}
contract HttpJsonScript is HttpTest {{
    function run() external {{
        string[] memory headers = new string[](1);
        headers[0] = vm.envString("HTTP_AUTHORIZATION");
        Http.HttpResponse memory response = http.httpRequest(Http.HttpRequest(
            "POST", string.concat(endpoint, "/api/echo?key=", vm.envString("HTTP_QUERY")),
            headers, bytes(vm.envString("HTTP_BODY"))
        ));
        require(response.status == 201);
        require(http.httpGet(string.concat(endpoint, "/api/price")).status == 200);
    }}
}}
"#
        ),
    );
    cmd.env("HTTP_ENDPOINT", &endpoint);
    cmd.env("HTTP_AUTHORIZATION", "Authorization: Bearer secret-token");
    cmd.env("HTTP_QUERY", "query-secret");
    cmd.env("HTTP_BODY", "body-secret");
    let output = cmd
        .args(["script", "HttpJsonScript", "--json"])
        .assert_success()
        .get_output()
        .stdout
        .clone();
    let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
    // Assert the exported HTTP nodes, including their raw ABI fields, not merely their display.
    let http_traces = output["traces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|trace| trace[1]["arena"].as_array().unwrap())
        .map(|node| &node["trace"])
        .filter(|trace| {
            trace["decoded"]["call_data"]["signature"]
                .as_str()
                .is_some_and(|signature| signature.starts_with("http"))
        })
        .map(|trace| {
            serde_json::json!({
                "data": trace["data"], "output": trace["output"], "decoded": trace["decoded"]
            })
        })
        .collect::<Vec<_>>();
    snapbox::assert_data_eq!(serde_json::to_string(&http_traces).unwrap(), str![[r#"
[
  {"data":"0x03ff4ec7","output":"0x","decoded":{"label":"VM","call_data":{"signature":"httpRequest((string,string,string[],bytes))","args":["POST http://127.0.0.1:[..]"]},"return_data":"<http response>"}},
  {"data":"0x52792547","output":"0x","decoded":{"label":"VM","call_data":{"signature":"httpGet(string)","args":["GET http://127.0.0.1:[..]"]},"return_data":"<http response>"}}
]
"#]].is_json());
}

#[forgetest_init]
async fn http_cheatcodes_reject_allowlisted_cross_origin_redirect(prj: _, cmd: _) {
    let (destination, requests) = server().await;
    let target = format!("{destination}/api/price");
    let app = Router::new().route(
        "/api/redirect",
        get(move || {
            let target = target.clone();
            async move { Redirect::temporary(&target) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    prj.update_config(|config| {
        config.http_allowlist = vec![format!("{endpoint}/api/"), format!("{destination}/api/")];
    });
    prj.add_test("Http.t.sol", TEST);
    cmd.env("HTTP_ENDPOINT", &endpoint);
    cmd.args(["test", "--mt", "test_cross_origin_redirect_is_not_followed"]).assert_success();
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}
