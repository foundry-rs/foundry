"""Loopback-only JSON-RPC transport for the frozen historical BAL benchmark."""

from collections import Counter
import gzip
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import socket
import threading


STATE_METHODS = {
    "eth_getBalance", "eth_getCode", "eth_getTransactionCount", "eth_getStorageAt",
    "eth_getAccountInfo", "eth_getAccount",
}


def write_json(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def rpc_error(payload, message, code=-32099):
    return {"jsonrpc": "2.0", "id": payload.get("id"),
            "error": {"code": code, "message": message}}


class Journal:
    def __init__(self, root):
        self.root = Path(root)
        self.lock = threading.Lock()

    def append(self, name, value):
        with self.lock, (self.root / name).open("a") as stream:
            stream.write(json.dumps(value, separators=(",", ":")) + "\n")


def request_key(method, params):
    # Hex case is insignificant for addresses, hashes, quantities and storage keys.
    def normalize(value):
        if isinstance(value, str) and value.startswith("0x"):
            return value.lower()
        if isinstance(value, list):
            return [normalize(item) for item in value]
        if isinstance(value, dict):
            return {key: normalize(item) for key, item in value.items()}
        return value

    params = normalize(params)
    if method == "eth_getStorageAt" and len(params) == 3:
        params[1] = hex(int(params[1], 16))
    return json.dumps([method, params], sort_keys=True, separators=(",", ":"))


class RecordedFixture:
    """Serve captured responses, with an irreversible barrier before native runs."""

    def __init__(self, path, journal):
        with gzip.open(path, "rt") as stream:
            fixture = json.load(stream)
        if fixture.get("schema_version") != 1:
            raise ValueError("Unsupported RPC fixture version")
        self.block_number = fixture["block_number"]
        self.parent_hash = fixture["parent_hash"]
        self.journal = journal
        self.records = {}
        self.lock = threading.Lock()
        self.phase = "metadata"
        self.offline = False
        self.counts = Counter()
        self.barrier_counts = Counter()
        self.failures = []
        self.state_requests = []
        for record in fixture["records"]:
            key = self.key(record["method"], record["params"])
            if key in self.records and self.records[key] != record["result"]:
                raise ValueError(f"Conflicting recorded response: {key}")
            self.records[key] = record["result"]
        # AccountInfo field projections retain authentic parent values; BAL is never state.
        for record in fixture["records"]:
            if record["method"] == "eth_getAccountInfo" and self.is_parent_state(record["params"]):
                for method, field in (("eth_getBalance", "balance"), ("eth_getCode", "code"),
                                      ("eth_getTransactionCount", "nonce")):
                    key = self.key(method, record["params"])
                    value = record["result"][field]
                    if key in self.records and self.records[key] != value:
                        raise ValueError(f"Conflicting captured account field: {key}")
                    self.records[key] = value

    def key(self, method, params):
        params = list(params)
        if method in STATE_METHODS and self.is_parent_state(params):
            params[-1] = self.parent_hash
        return request_key(method, params)

    def get(self, method, params):
        key = self.key(method, params)
        if (key not in self.records and method == "eth_getAccountInfo"
                and self.is_parent_state(params)):
            fields = {field: self.records.get(self.key(part, params)) for part, field in (
                ("eth_getBalance", "balance"), ("eth_getCode", "code"),
                ("eth_getTransactionCount", "nonce"))}
            if all(value is not None for value in fields.values()):
                return fields
        if key not in self.records:
            raise ValueError(f"Missing captured RPC response: {key}")
        return self.records[key]

    def close_barrier(self):
        with self.lock:
            self.phase = "native-runner-offline"
            self.offline = True

    def is_parent_state(self, params):
        tag = params[-1] if params else None
        return (tag in (hex(self.block_number - 1), self.parent_hash)
                or isinstance(tag, dict) and tag.get("blockHash") == self.parent_hash)

    def dispatch(self, payload):
        with self.lock:
            method = payload["method"]
            params = payload.get("params", [])
            row = {"phase": self.phase, "request": payload}
            self.counts[method] += 1
            if method in STATE_METHODS:
                self.state_requests.append({"method": method, "params": params})
            if self.offline:
                self.barrier_counts[method] += 1
                row["source"] = "offline-barrier"
                response = rpc_error(payload, "Unexpected fixture request after preparation")
                self.failures.append(row)
            elif method in STATE_METHODS and not self.is_parent_state(params):
                row["source"] = "invalid-state-block"
                response = rpc_error(payload, "State request must target authentic parent")
                self.failures.append(row)
            else:
                try:
                    result = self.get(method, params)
                    response = {"jsonrpc": "2.0", "id": payload.get("id"), "result": result}
                    row["source"] = "captured-response"
                except ValueError as error:
                    row["source"] = "missing-captured-response"
                    response = rpc_error(payload, str(error))
                    self.failures.append(row)
            row["response"] = response
            self.journal.append("fixture-events.jsonl", row)
            return response


class LocalClient:
    """Reuse HTTP connections without allowing non-loopback destinations."""

    def __init__(self):
        self.local = threading.local()
        self.lock = threading.Lock()
        self.connections = []

    def request(self, port, payload):
        connections = getattr(self.local, "connections", None)
        if connections is None:
            connections = self.local.connections = {}
        if port not in connections:
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=55)
            connections[port] = connection
            with self.lock:
                self.connections.append(connection)
        connection = connections[port]
        try:
            connection.request("POST", "/", json.dumps(payload),
                               {"Content-Type": "application/json"})
            response = connection.getresponse()
            body = response.read()
            if response.status != 200:
                raise RuntimeError(f"Local RPC returned HTTP {response.status}")
            return json.loads(body)
        except Exception:
            connection.close()
            del connections[port]
            raise

    def rpc(self, port, method, params):
        result = self.request(port, {"jsonrpc": "2.0", "id": 1,
                                     "method": method, "params": params})
        if "error" in result:
            raise RuntimeError(json.dumps(result["error"]))
        return result["result"]

    def close(self):
        with self.lock:
            for connection in self.connections:
                connection.close()
            self.connections.clear()


class RpcServer(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 128


def start_server(dispatch, journal):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def handle(self):
            try:
                super().handle()
            except ConnectionResetError:
                # Clients may close an idle keep-alive connection with a reset.
                pass

        def do_POST(self):
            payload = None
            try:
                payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                result = ([dispatch(item) for item in payload] if isinstance(payload, list)
                          else dispatch(payload))
            except Exception as error:
                journal.append("transport-errors.jsonl", {"request": payload, "error": str(error)})
                result = rpc_error(payload if isinstance(payload, dict) else {}, str(error), -32098)
            body = json.dumps(result, separators=(",", ":")).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def log_message(self, *args):
            pass

    server = RpcServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def free_port():
    with socket.socket() as connection:
        connection.bind(("127.0.0.1", 0))
        return connection.getsockname()[1]
