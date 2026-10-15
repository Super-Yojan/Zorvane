"""Embedded loopback gateway for the browser simulator.

The wire format matches `zorvane-browser-protocol`: tagged JSON control
messages, and publications as a little-endian header length, a JSON header,
then the payload bytes.
"""

from __future__ import annotations

import hashlib
import hmac
import json
import os
import socket
import threading
import time
import urllib.error
import urllib.request
from urllib.parse import urljoin, urlsplit, urlunsplit

VERSION = 1
MAX_PACKET = 17 * 1024 * 1024
HEARTBEAT_S = 0.1
LEASE_S = 0.5
PREFIX = "terra/rover"
_WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
_PLAY_PREFIX = "/Zorvane/play/webgpu/"
_DEFAULT_UPSTREAM = "https://super-yojan.dev/Zorvane/play/webgpu/"


class BrowserHostConfig:
    def __init__(
        self,
        bind_host: str = "127.0.0.1",
        bind_port: int = 0,
        play_url: str = _DEFAULT_UPSTREAM,
        recording_dir: str = "runs",
        force_proxy: bool = False,
    ) -> None:
        self.bind_host = bind_host
        self.bind_port = bind_port
        self.play_url = play_url
        self.recording_dir = recording_dir
        self.force_proxy = force_proxy


def tokens_match(candidate: str, token: str) -> bool:
    return hmac.compare_digest(candidate.encode(), token.encode())


def _loopback_http(url: str) -> bool:
    parts = urlsplit(url)
    if parts.scheme != "http" or parts.username or parts.password:
        return False
    host = (parts.hostname or "").lower()
    return host in {"localhost", "127.0.0.1", "::1"}


def _directory_prefix(url: str) -> str:
    path = urlsplit(url).path or "/"
    return path if path.endswith("/") else path + "/"


def asset_relative(target: str, prefix: str) -> tuple[bool, str] | None:
    """Return `(redirect, relative)`, or None when the target is rejected."""
    if target.startswith("//") or "://" in target:
        return None
    path, question, query = target.partition("?")
    if (
        not path.startswith("/")
        or "\\" in path
        or any(ord(char) < 32 for char in path + query)
        or " " in query
    ):
        return None
    parts: list[str] = []
    for segment in path.split("/"):
        if segment in {"", "."}:
            continue
        if segment == ".." or _decoded_unsafe(segment):
            return None
        parts.append(segment)
    normalized = "/" + "/".join(parts)
    if path.endswith("/") and normalized != "/":
        normalized += "/"
    bare = prefix.rstrip("/")
    if normalized in {bare, "/"}:
        return True, ""
    if not normalized.startswith(prefix):
        return None
    relative = normalized[len(prefix) :]
    if question:
        relative += "?" + query
    return False, relative


def _decoded_unsafe(segment: str) -> bool:
    raw = segment.encode()
    out = bytearray()
    index = 0
    while index < len(raw):
        if raw[index] == ord("%") and index + 2 < len(raw):
            try:
                out.append(int(raw[index + 1 : index + 3], 16))
                index += 3
                continue
            except ValueError:
                pass
        out.append(raw[index])
        index += 1
    try:
        text = out.decode()
    except UnicodeDecodeError:
        return True
    return "/" in text or "\\" in text or text == ".." or "\0" in text


class BrowserHost:
    def __init__(self, config: BrowserHostConfig) -> None:
        if config.bind_host not in {"127.0.0.1", "localhost", "::1"}:
            raise ValueError("browser host must bind to a loopback address")
        parts = urlsplit(config.play_url)
        if parts.scheme not in {"https", "http"} or parts.username or parts.password:
            raise ValueError("play_url must use HTTPS or loopback HTTP")
        if parts.scheme == "http" and not _loopback_http(config.play_url):
            raise ValueError("play_url must use HTTPS or loopback HTTP")
        self._proxy = parts.scheme == "https" or config.force_proxy
        self._upstream = config.play_url if self._proxy else None
        self._recording_dir = config.recording_dir
        self._token = os.urandom(32).hex()
        self._stop = threading.Event()
        self._connected = threading.Event()
        self._state_lock = threading.Lock()
        self._subscriptions: list[tuple[str, object]] = []
        self._revision = 0
        self._outgoing: list[bytes] = []
        self._fatal: str | None = None
        self._session = threading.Lock()
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind((config.bind_host if config.bind_host != "localhost" else "127.0.0.1", config.bind_port))
        listener.listen(16)
        listener.settimeout(0.2)
        self._listener = listener
        host, port = listener.getsockname()
        endpoint = f"ws://{host}:{port}/bridge"
        fragment = f"endpoint={_query(endpoint)}&token={self._token}"
        if self._proxy:
            prefix = _directory_prefix(config.play_url)
            page = f"http://{host}:{port}{prefix}"
            self._origin = f"http://{host}:{port}"
        else:
            page = config.play_url if config.play_url.endswith("/") or "?" in config.play_url else config.play_url
            origin = urlsplit(config.play_url)
            port_text = f":{origin.port}" if origin.port else ""
            self._origin = f"{origin.scheme}://{origin.hostname}{port_text}"
            page = urlunsplit((origin.scheme, origin.netloc, origin.path or "/", "", ""))
        self.pairing_url = f"{page}#{fragment}"
        self._prefix = _directory_prefix(config.play_url) if self._proxy else ""
        self._thread = threading.Thread(target=self._accept_loop, name="zorvane-browser-host", daemon=True)
        self._thread.start()

    def subscribe_pose(self, rover_id: int, callback) -> None:
        topic = f"{PREFIX}/{int(rover_id)}/pose"

        def deliver(payload: bytes) -> None:
            from .client import parse_pose

            try:
                pose = parse_pose(payload)
            except (KeyError, TypeError, ValueError, json.JSONDecodeError):
                return
            callback(pose)

        self._subscribe(topic, deliver)

    def set_velocity(self, rover_id: int, linear: float, angular: float) -> None:
        import math

        limit = 3.4028234663852886e38
        if not math.isfinite(linear) or not math.isfinite(angular) or abs(linear) > limit or abs(angular) > limit:
            raise ValueError("velocity must be finite float32 values")
        body = json.dumps({"linear": linear, "angular": angular}, separators=(",", ":")).encode()
        self._publish(f"{PREFIX}/{int(rover_id)}/cmd_vel", "application/json", body)

    def wait_connected(self, timeout: float) -> None:
        if not self._connected.wait(timeout):
            raise TimeoutError(f"browser did not pair within {timeout}s")

    def close(self) -> None:
        self._stop.set()
        try:
            self._listener.close()
        except OSError:
            pass
        self._thread.join(timeout=2)

    def _subscribe(self, expression: str, callback) -> None:
        with self._state_lock:
            if len(self._subscriptions) >= 256:
                raise RuntimeError("too many subscriptions")
            self._subscriptions.append((expression, callback))
            self._revision += 1

    def _publish(self, topic: str, encoding: str, payload: bytes) -> None:
        with self._state_lock:
            if not self._connected.is_set() or self._fatal is not None:
                raise RuntimeError("browser is not connected")
            if len(self._outgoing) >= 512:
                self._fatal = "publication queue full"
                raise RuntimeError(self._fatal)
            self._outgoing.append(_encode_publication(topic, encoding, payload))

    def _accept_loop(self) -> None:
        while not self._stop.is_set():
            try:
                connection, _ = self._listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            threading.Thread(target=self._dispatch, args=(connection,), daemon=True).start()

    def _dispatch(self, connection: socket.socket) -> None:
        try:
            head = _peek_headers(connection, self._stop)
        except OSError:
            connection.close()
            return
        if head is None:
            connection.close()
            return
        if not _is_websocket(head):
            if self._upstream is not None:
                _serve_http(connection, self._upstream, self._prefix)
            else:
                connection.close()
            return
        connected = self._connected.is_set()
        acquired = self._session.acquire(blocking=not connected)
        if not acquired:
            connection.close()
            return
        try:
            self._serve_socket(connection)
        finally:
            self._connected.clear()
            with self._state_lock:
                self._outgoing.clear()
                self._fatal = None
            self._session.release()
            try:
                connection.close()
            except OSError:
                pass

    def _serve_socket(self, connection: socket.socket) -> None:
        connection.settimeout(0.5)
        if not _handshake(connection, self._origin, f"{self._listener.getsockname()[0]}:{self._listener.getsockname()[1]}"):
            return
        try:
            opcode, payload = _read_frame(connection)
        except (OSError, ValueError):
            _send_text(connection, {"type": "error", "message": "pairing message could not be read"})
            return
        text = payload.decode(errors="replace") if opcode == 1 else ""
        if opcode != 1 or not _hello_ok(text, self._token):
            _send_text(connection, {"type": "error", "message": "invalid pairing secret or protocol version"})
            return
        _send_text(connection, {"type": "ready", "version": VERSION})
        self._connected.set()
        last_received = time.monotonic()
        last_sent = time.monotonic()
        revision = -1
        run_id = None
        recorder = None
        connection.settimeout(0.05)
        while not self._stop.is_set():
            if time.monotonic() - last_received >= LEASE_S:
                return
            with self._state_lock:
                if self._fatal is not None:
                    return
            try:
                opcode, payload = _read_frame(connection)
            except socket.timeout:
                opcode = None
            except (OSError, ValueError):
                return
            if opcode == 1:
                if len(payload) > 1024 * 1024:
                    return
                try:
                    message = json.loads(payload.decode())
                except json.JSONDecodeError:
                    return
                if not isinstance(message, dict):
                    return
                kind = message.get("type")
                if kind == "heartbeat" and set(message) == {"type"}:
                    last_received = time.monotonic()
                elif kind == "manifest" and run_id is None and set(message) == {"type", "run_id", "manifest"}:
                    run_id = message["run_id"]
                    recorder = _open_recorder(self._recording_dir, message["manifest"])
                    _send_text(connection, {"type": "recorder", "healthy": recorder is not None})
                elif kind == "record" and set(message) == {"type", "event"}:
                    event = message["event"]
                    if not isinstance(event, dict) or event.get("run_id") != run_id:
                        return
                    if recorder is not None:
                        recorder.write(json.dumps(event, separators=(",", ":")) + "\n")
                        recorder.flush()
                else:
                    return
            elif opcode == 2:
                topic, _encoding, body = _decode_publication(payload)
                with self._state_lock:
                    callbacks = [callback for expression, callback in self._subscriptions if _matches(expression, topic)]
                for callback in callbacks:
                    callback(body)
                last_received = time.monotonic()
            elif opcode == 8:
                return
            elif opcode == 9:
                _send_frame(connection, 10, payload)
            elif opcode == 10 or opcode is None:
                pass
            else:
                return
            with self._state_lock:
                current = self._revision
                topics = sorted({expression for expression, _callback in self._subscriptions})
                pending = list(self._outgoing)
                self._outgoing.clear()
            if current != revision:
                revision = current
                _send_text(connection, {"type": "subscribe", "topics": topics})
                last_sent = time.monotonic()
            if time.monotonic() - last_sent >= HEARTBEAT_S:
                _send_text(connection, {"type": "heartbeat"})
                if run_id is not None:
                    healthy = recorder is not None and not recorder.closed
                    _send_text(connection, {"type": "recorder", "healthy": healthy})
                last_sent = time.monotonic()
            for frame in pending:
                _send_frame(connection, 2, frame)


def _query(value: str) -> str:
    return value.replace("%", "%25").replace("&", "%26").replace("=", "%3D").replace("+", "%2B")


def _hello_ok(text: str, token: str) -> bool:
    if len(text) > 4096:
        return False
    try:
        message = json.loads(text)
    except json.JSONDecodeError:
        return False
    if not isinstance(message, dict) or set(message) != {"type", "version", "token"}:
        return False
    if message["type"] != "hello" or message["version"] != VERSION or not isinstance(message["token"], str):
        return False
    return tokens_match(message["token"], token)


def _open_recorder(directory: str, manifest: object):
    try:
        os.makedirs(directory, exist_ok=True)
        name = os.urandom(12).hex()
        handle = open(os.path.join(directory, f"browser-{name}.jsonl"), "w", encoding="utf-8")
        record = {"kind": "manifest", "manifest": manifest}
        handle.write(json.dumps(record, separators=(",", ":")) + "\n")
        handle.flush()
        return handle
    except OSError:
        return None


def _encode_publication(topic: str, encoding: str, payload: bytes) -> bytes:
    header = json.dumps({"topic": topic, "encoding": encoding}, separators=(",", ":")).encode()
    if len(header) > 4096 or 4 + len(header) + len(payload) > MAX_PACKET:
        raise ValueError("publication too large")
    return len(header).to_bytes(4, "little") + header + payload


def _decode_publication(payload: bytes) -> tuple[str, str, bytes]:
    if len(payload) < 4 or len(payload) > MAX_PACKET:
        raise ValueError("invalid publication size")
    size = int.from_bytes(payload[:4], "little")
    if size > 4096 or size > len(payload) - 4:
        raise ValueError("invalid publication header size")
    header = json.loads(payload[4 : 4 + size])
    if set(header) != {"topic", "encoding"}:
        raise ValueError("invalid publication header")
    return header["topic"], header["encoding"], payload[4 + size :]


def _matches(expression: str, topic: str) -> bool:
    def match(expected: list[str], actual: list[str]) -> bool:
        if not expected:
            return not actual
        if expected[0] == "**":
            return match(expected[1:], actual) or (bool(actual) and match(expected, actual[1:]))
        if not actual:
            return False
        return (expected[0] in {"*", actual[0]}) and match(expected[1:], actual[1:])

    return match(expression.split("/"), topic.split("/"))


def _peek_headers(connection: socket.socket, stop: threading.Event) -> bytes | None:
    connection.settimeout(0.05)
    buffer = b""
    deadline = time.monotonic() + 0.5
    while time.monotonic() < deadline and not stop.is_set():
        try:
            chunk = connection.recv(4096, socket.MSG_PEEK)
        except socket.timeout:
            continue
        if not chunk:
            return None
        buffer = chunk
        if b"\r\n\r\n" in buffer:
            return buffer
    return None


def _is_websocket(head: bytes) -> bool:
    text = head.decode(errors="replace")
    lines = text.split("\r\n")
    if not lines:
        return False
    parts = lines[0].split()
    if len(parts) < 2 or parts[0] != "GET":
        return False
    path = parts[1].split("?", 1)[0]
    if path != "/bridge":
        return False
    return any(line.lower().startswith("upgrade:") and line.split(":", 1)[1].strip().lower() == "websocket" for line in lines)


def _handshake(connection: socket.socket, origin: str, host: str) -> bool:
    data = b""
    while b"\r\n\r\n" not in data and len(data) < 8192:
        chunk = connection.recv(4096)
        if not chunk:
            return False
        data += chunk
    text = data.decode(errors="replace")
    lines = text.split("\r\n")
    headers = {}
    for line in lines[1:]:
        if not line or ":" not in line:
            continue
        name, value = line.split(":", 1)
        headers[name.lower()] = value.strip()
    request = lines[0].split()
    path = request[1].split("?", 1)[0] if len(request) > 1 else ""
    if path != "/bridge" or headers.get("origin") != origin or headers.get("host") != host:
        body = b"pairing origin rejected"
        connection.sendall(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: "
            + str(len(body)).encode()
            + b"\r\nConnection: close\r\n\r\n"
            + body
        )
        return False
    key = headers.get("sec-websocket-key", "")
    accept = hashlib.sha1((key + _WS_GUID).encode()).digest()
    import base64

    response = (
        "HTTP/1.1 101 Switching Protocols\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Accept: {base64.b64encode(accept).decode()}\r\n\r\n"
    )
    connection.sendall(response.encode())
    return True


def _read_exact(connection: socket.socket, count: int) -> bytes:
    data = b""
    while len(data) < count:
        chunk = connection.recv(count - len(data))
        if not chunk:
            raise OSError("socket closed")
        data += chunk
    return data


def _read_frame(connection: socket.socket) -> tuple[int, bytes]:
    first = _read_exact(connection, 2)
    opcode = first[0] & 0x0F
    length = first[1] & 0x7F
    if length == 126:
        length = int.from_bytes(_read_exact(connection, 2), "big")
    elif length == 127:
        length = int.from_bytes(_read_exact(connection, 8), "big")
    if length > MAX_PACKET:
        raise ValueError("frame too large")
    mask = _read_exact(connection, 4) if first[1] & 0x80 else None
    payload = _read_exact(connection, length) if length else b""
    if mask is not None:
        payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
    return opcode, payload


def _send_frame(connection: socket.socket, opcode: int, payload: bytes) -> None:
    header = bytearray([0x80 | opcode])
    length = len(payload)
    if length < 126:
        header.append(length)
    elif length < 65536:
        header.append(126)
        header.extend(length.to_bytes(2, "big"))
    else:
        header.append(127)
        header.extend(length.to_bytes(8, "big"))
    connection.sendall(bytes(header) + payload)


def _send_text(connection: socket.socket, message: dict) -> None:
    _send_frame(connection, 1, json.dumps(message, separators=(",", ":")).encode())


def _serve_http(connection: socket.socket, upstream: str, prefix: str) -> None:
    try:
        connection.settimeout(2)
        data = b""
        while b"\r\n\r\n" not in data and len(data) < 8192:
            chunk = connection.recv(4096)
            if not chunk:
                return
            data += chunk
        text = data.decode(errors="replace")
        lines = text.split("\r\n")
        parts = lines[0].split()
        if len(parts) < 2 or parts[0] not in {"GET", "HEAD"}:
            _http(connection, 405, b"method not allowed", "text/plain")
            return
        route = asset_relative(parts[1], prefix)
        if route is None:
            _http(connection, 404, b"not found", "text/plain")
            return
        redirect, relative = route
        if redirect:
            body = b"redirect"
            header = (
                "HTTP/1.1 302 Found\r\n"
                f"Location: {prefix}\r\n"
                f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n"
            )
            connection.sendall(header.encode() + body)
            return
        base = upstream if upstream.endswith("/") else upstream + "/"
        target = urljoin(base, relative)
        upstream_parts = urlsplit(base)
        target_parts = urlsplit(target)
        if (target_parts.scheme, target_parts.hostname, target_parts.port) != (
            upstream_parts.scheme,
            upstream_parts.hostname,
            upstream_parts.port,
        ):
            _http(connection, 404, b"not found", "text/plain")
            return
        request = urllib.request.Request(
            target,
            method=parts[0],
            headers={"Accept-Encoding": "identity", "User-Agent": "Zorvane/0.1"},
        )
        for line in lines[1:]:
            if line.lower().startswith("range:"):
                request.add_header("Range", line.split(":", 1)[1].strip())
        document = relative in {"", "index.html"} or relative.startswith("index.html?")
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                _forward(connection, response, parts[0] == "HEAD")
        except urllib.error.HTTPError as error:
            if document and error.code == 404 and parts[0] != "HEAD":
                _http(connection, 502, _unavailable(upstream), "text/html; charset=utf-8")
            else:
                body = error.read(1024 * 1024)
                content_type = error.headers.get("Content-Type", "text/plain")
                _http(connection, error.code, body, content_type)
        except urllib.error.URLError:
            if document and parts[0] != "HEAD":
                _http(connection, 502, _unavailable(upstream), "text/html; charset=utf-8")
            else:
                _http(connection, 502, b"simulator page unavailable", "text/plain")
    except OSError:
        return
    finally:
        try:
            connection.close()
        except OSError:
            pass


def _forward(connection: socket.socket, response, head: bool) -> None:
    content_type = response.headers.get("Content-Type", "application/octet-stream")
    length = response.headers.get("Content-Length")
    status = getattr(response, "status", 200)
    extra = f"Content-Type: {content_type}\r\n"
    if length is not None:
        extra += f"Content-Length: {length}\r\n"
    header = f"HTTP/1.1 {status} OK\r\n{extra}Connection: close\r\n\r\n"
    connection.sendall(header.encode())
    if head:
        return
    while True:
        chunk = response.read(65536)
        if not chunk:
            break
        connection.sendall(chunk)


def _unavailable(upstream: str) -> bytes:
    safe = upstream.replace("&", "&amp;").replace("<", "&lt;")
    return (
        "<!DOCTYPE html><html lang=\"en\"><meta charset=\"utf-8\"><title>Zorvane</title>"
        f"<p>The WebGPU simulator could not be loaded from {safe}.</p>"
        "<p>This client is running. The published WebGPU build is not available yet.</p></html>"
    ).encode()


def _http(connection: socket.socket, status: int, body: bytes, content_type: str) -> None:
    header = (
        f"HTTP/1.1 {status} Error\r\nContent-Type: {content_type}\r\n"
        f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n"
    )
    connection.sendall(header.encode() + body)
