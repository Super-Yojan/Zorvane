import json
import socket
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

from zorvane import connect
from zorvane.browser import BrowserHostConfig, asset_relative, tokens_match
from zorvane.client import Client


def _frame(opcode: int, payload: bytes) -> bytes:
    mask = b"\x01\x02\x03\x04"
    masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
    header = bytearray([0x80 | opcode])
    length = len(payload)
    if length < 126:
        header.append(0x80 | length)
    else:
        header.append(0x80 | 126)
        header.extend(length.to_bytes(2, "big"))
    header.extend(mask)
    return bytes(header) + masked


def _read_frame(sock: socket.socket) -> tuple[int, bytes]:
    header = _read_exact(sock, 2)
    opcode = header[0] & 0x0F
    length = header[1] & 0x7F
    if length == 126:
        length = int.from_bytes(_read_exact(sock, 2), "big")
    elif length == 127:
        length = int.from_bytes(_read_exact(sock, 8), "big")
    payload = _read_exact(sock, length) if length else b""
    return opcode, payload


def _read_exact(sock: socket.socket, count: int) -> bytes:
    data = b""
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise OSError("closed")
        data += chunk
    return data


class Socket:
    def __init__(self, pairing: str, token: str | None = None, origin: str | None = None):
        page = urlsplit(pairing)
        fields = parse_qs(page.fragment)
        endpoint = urlsplit(fields["endpoint"][0])
        self.token = token if token is not None else fields["token"][0]
        self._sock = socket.create_connection((endpoint.hostname, endpoint.port), timeout=2)
        self._sock.settimeout(2)
        origin = origin or f"{page.scheme}://{page.hostname}:{page.port}"
        key = "dGhlIHNhbXBsZSBub25jZQ=="
        request = (
            f"GET /bridge HTTP/1.1\r\n"
            f"Host: {endpoint.hostname}:{endpoint.port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Origin: {origin}\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self._sock.sendall(request.encode())
        response = b""
        while b"\r\n\r\n" not in response:
            chunk = self._sock.recv(4096)
            if not chunk:
                break
            response += chunk
        self.status = response.split(b"\r\n", 1)[0].decode()

    def hello(self) -> None:
        body = json.dumps({"type": "hello", "version": 1, "token": self.token}).encode()
        self._sock.sendall(_frame(1, body))

    def text(self) -> dict:
        while True:
            opcode, payload = _read_frame(self._sock)
            if opcode == 1:
                message = json.loads(payload)
                if message.get("type") == "heartbeat":
                    self._sock.sendall(_frame(1, b'{"type":"heartbeat"}'))
                    continue
                return message
            if opcode == 2:
                return {"binary": payload}

    def send_pose(self) -> None:
        body = b'{"x":3,"y":-4,"heading":0.5}'
        header = json.dumps(
            {"topic": "terra/rover/0/pose", "encoding": "application/json"},
            separators=(",", ":"),
        ).encode()
        packet = len(header).to_bytes(4, "little") + header + body
        self._sock.sendall(_frame(2, packet))

    def close(self) -> None:
        self._sock.close()


class BrowserTests(unittest.TestCase):
    def test_tokens_match_rejects_a_different_secret(self):
        self.assertTrue(tokens_match("ab", "ab"))
        self.assertFalse(tokens_match("ab", "ac"))
        self.assertFalse(tokens_match("a", "ab"))

    def test_asset_paths_stay_under_the_play_prefix(self):
        prefix = "/Zorvane/play/webgpu/"
        self.assertEqual(asset_relative("/Zorvane/play/webgpu/", prefix), (False, ""))
        self.assertEqual(asset_relative("/", prefix), (True, ""))
        self.assertEqual(asset_relative("/Zorvane/play/webgpu/app.js", prefix), (False, "app.js"))
        self.assertIsNone(asset_relative("/Zorvane/play/webgpu/../../secret", prefix))
        self.assertIsNone(asset_relative("/Zorvane/play/webgpu/%2e%2e/secret", prefix))

    def test_connect_prints_a_loopback_pairing_url(self):
        import io
        from contextlib import redirect_stderr

        buffer = io.StringIO()
        with redirect_stderr(buffer):
            client = connect()
        try:
            page = urlsplit(client.pairing_url)
            self.assertEqual(page.scheme, "http")
            self.assertEqual(page.hostname, "127.0.0.1")
            self.assertIn(client.pairing_url, buffer.getvalue())
        finally:
            client.close()

    def test_pose_and_velocity_round_trip(self):
        client = Client.host_browser()
        received = []
        client.subscribe_pose(0, received.append)
        browser = Socket(client.pairing_url)
        browser.hello()
        self.assertEqual(browser.text()["type"], "ready")
        self.assertEqual(browser.text()["topics"], ["terra/rover/0/pose"])
        client.wait_connected(1)
        browser.send_pose()
        deadline = __import__("time").monotonic() + 1
        while not received and __import__("time").monotonic() < deadline:
            __import__("time").sleep(0.01)
        self.assertEqual((received[0].x, received[0].y, received[0].heading), (3.0, -4.0, 0.5))
        client.set_velocity(0, 0.4, -0.2)
        message = browser.text()
        self.assertIn("binary", message)
        payload = message["binary"]
        size = int.from_bytes(payload[:4], "little")
        header = json.loads(payload[4 : 4 + size])
        body = json.loads(payload[4 + size :])
        self.assertEqual(header["topic"], "terra/rover/0/cmd_vel")
        self.assertEqual(body, {"linear": 0.4, "angular": -0.2})
        browser.close()
        client.close()

    def test_wrong_secret_and_origin_are_rejected(self):
        client = Client.host_browser()
        browser = Socket(client.pairing_url, token="nope")
        browser.hello()
        self.assertEqual(browser.text()["type"], "error")
        browser.close()
        with self.assertRaises(RuntimeError):
            client.set_velocity(0, 1.0, 0.0)
        rejected = Socket(client.pairing_url, origin="https://untrusted.invalid")
        self.assertIn("403", rejected.status)
        rejected.close()
        client.close()

    def test_gateway_serves_proxied_page(self):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                body = b"page-ok" if self.path.rstrip("/").endswith("webgpu") or self.path.endswith("/") else b"script-ok"
                if self.path.endswith("app.js"):
                    body = b"script-ok"
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, fmt, *args):
                del fmt, args

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        upstream = f"http://127.0.0.1:{server.server_address[1]}/Zorvane/play/webgpu/"
        client = Client.host_browser(BrowserHostConfig(play_url=upstream, force_proxy=True))
        page = urlsplit(client.pairing_url)
        try:
            body = _http_get(f"http://127.0.0.1:{page.port}/Zorvane/play/webgpu/")
            self.assertIn(b"page-ok", body)
            script = _http_get(f"http://127.0.0.1:{page.port}/Zorvane/play/webgpu/app.js")
            self.assertIn(b"script-ok", script)
            escaped = _http_get(f"http://127.0.0.1:{page.port}/Zorvane/play/webgpu/%2e%2e/secret")
            self.assertTrue(escaped.startswith(b"HTTP/1.1 404"))
        finally:
            client.close()
            server.shutdown()


def _http_get(url: str) -> bytes:
    parts = urlsplit(url)
    sock = socket.create_connection((parts.hostname, parts.port), timeout=2)
    sock.sendall(f"GET {parts.path} HTTP/1.1\r\nHost: {parts.hostname}\r\nConnection: close\r\n\r\n".encode())
    data = b""
    while True:
        chunk = sock.recv(4096)
        if not chunk:
            break
        data += chunk
    sock.close()
    return data


if __name__ == "__main__":
    unittest.main()
