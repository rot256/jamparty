"""What the browser tests share: running `jamparty` processes, joining a
share, talking to the joiner's local server, the fixture sites and dev
servers, the overlay and pages."""

import base64
import hashlib
import http.client
import json
import os
import queue
import re
import socket
import subprocess
import tempfile
import threading
import time
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

import pytest
from playwright.sync_api import Browser, Locator, Page, WebSocketRoute, expect

ROOT = Path(__file__).resolve().parent.parent
TARGET_DIR = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
BINARY = TARGET_DIR / "debug/jamparty"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
TIMEOUT = 20.0
SITE = ROOT / "tests/fixtures/site"
SPA = ROOT / "tests/fixtures/spa"
FIXTURE_HTML = (SITE / "index.html").read_bytes()


class Process:
    """A jamparty process whose output lines are collected as they arrive."""

    def __init__(
        self, arguments: list[str], env: dict[str, str], stdin: str | None = None
    ) -> None:
        self.process: subprocess.Popen[str] = subprocess.Popen(
            [str(BINARY), *arguments],
            cwd=ROOT,
            env=env,
            stdin=subprocess.DEVNULL if stdin is None else subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        if stdin is not None:
            pipe = self.process.stdin
            assert pipe is not None
            _ = pipe.write(stdin)
            pipe.close()
        self.lines: queue.Queue[str] = queue.Queue()
        self.seen: list[str] = []
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        stdout = self.process.stdout
        assert stdout is not None
        for line in stdout:
            self.lines.put(line.rstrip("\n"))

    def wait_for(self, prefix: str) -> str:
        """The rest of the first output line starting with `prefix`."""
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=0.1)
            except queue.Empty:
                if self.process.poll() is not None and self.lines.empty():
                    break
                continue
            self.seen.append(line)
            if line.startswith(prefix):
                return line.removeprefix(prefix)
        pytest.fail(f"no line starting with {prefix!r}; output: {self.seen}")

    def drain(self) -> list[str]:
        """The lines that arrived since the last look, without waiting."""
        new: list[str] = []
        while True:
            try:
                new.append(ANSI.sub("", self.lines.get_nowait()))
            except queue.Empty:
                break
        self.seen.extend(new)
        return new

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
        try:
            _ = self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            _ = self.process.wait(timeout=5)


@dataclass(frozen=True)
class Shared:
    token: str
    address: str
    process: Process


@dataclass(frozen=True)
class Joined:
    url: str
    host: str
    port: int


def runtime_dir(home: Path) -> Path:
    # Unix socket paths are short: keep the runtime directory out of the
    # (long) pytest directory.
    digest = hashlib.sha256(str(home).encode()).hexdigest()[:10]
    return Path(tempfile.gettempdir()) / f"jj-{digest}"


def environment(home: Path) -> dict[str, str]:
    return {
        **os.environ,
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / "config"),
        "XDG_RUNTIME_DIR": str(runtime_dir(home)),
    }


def jamparty(home: Path, *arguments: str) -> str:
    """Runs a short jamparty command and returns its output."""
    result = subprocess.run(
        [str(BINARY), *arguments],
        env=environment(home),
        capture_output=True,
        text=True,
        timeout=TIMEOUT,
        check=True,
    )
    return result.stdout


@contextmanager
def share(home: Path, *source: str) -> Iterator[Shared]:
    arguments = list(source)
    arguments.insert(
        arguments.index("--") if "--" in arguments else len(arguments), "--local"
    )
    process = Process(arguments, environment(home))
    try:
        token = process.wait_for("jamparty: token: ")
        address = process.wait_for("jamparty: direct address: ")
        yield Shared(token, address, process)
    finally:
        process.stop()


@contextmanager
def join_process(
    home: Path,
    shared: Shared,
    token: str,
    name: str | None = "alice",
    port: int | None = None,
) -> Iterator[Process]:
    # The token goes on stdin, as `jamparty join -` reads it: out of argv.
    arguments = ["join", "-", "--no-open"]
    if name is not None:
        arguments += ["--name", name]
    if port is not None:
        arguments += ["--port", str(port)]
    process = Process(
        [*arguments, "--share-addr", shared.address],
        environment(home),
        stdin=f"{token}\n",
    )
    try:
        yield process
    finally:
        process.stop()


@contextmanager
def join(
    home: Path, shared: Shared, name: str | None = "alice", port: int | None = None
) -> Iterator[Joined]:
    with join_process(home, shared, shared.token, name, port) as process:
        url = process.wait_for("jamparty: open ")
        parts = urlsplit(url)
        assert parts.hostname is not None and parts.port is not None
        joined = Joined(url, parts.netloc, parts.port)
        wait_connected(joined)
        yield joined


def request(
    joined: Joined,
    path: str,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    body: bytes | None = None,
) -> tuple[int, bytes]:
    connection = http.client.HTTPConnection("127.0.0.1", joined.port, timeout=TIMEOUT)
    try:
        connection.request(
            method, path, body=body, headers={"Host": joined.host, **(headers or {})}
        )
        response = connection.getresponse()
        return response.status, response.read()
    finally:
        connection.close()


def wait_connected(joined: Joined) -> None:
    def connected() -> bool:
        try:
            return cast(dict[str, object], hello(joined)["link"])["connected"] is True
        except OSError:
            return False

    eventually("the joiner to be admitted", connected)


class PageSocket:
    """The joiner's page socket (`api/socket`), spoken by hand: what a page
    on the joiner's origin is told, as an oracle for the tests."""

    def __init__(self, joined: Joined, origin: str | None = None) -> None:
        self.sock: socket.socket = socket.create_connection(
            ("127.0.0.1", joined.port), timeout=TIMEOUT
        )
        key = base64.b64encode(os.urandom(16)).decode()
        origin = f"http://{joined.host}" if origin is None else origin
        handshake = (
            f"GET /_jamparty/api/socket HTTP/1.1\r\nHost: {joined.host}\r\n"
            + "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            + f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n"
            + f"Origin: {origin}\r\n\r\n"
        )
        self.sock.sendall(handshake.encode())
        self.buffer: bytes = b""
        while b"\r\n\r\n" not in self.buffer:
            chunk = self.sock.recv(4096)
            if not chunk:
                break
            self.buffer += chunk
        head, _, self.buffer = self.buffer.partition(b"\r\n\r\n")
        self.status: int = int(head.split(b" ")[1]) if head else 0

    def _read(self, size: int) -> bytes:
        while len(self.buffer) < size:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise OSError("the socket closed")
            self.buffer += chunk
        data, self.buffer = self.buffer[:size], self.buffer[size:]
        return data

    def next(self) -> dict[str, object]:
        """The next message (the joiner's frames are unmasked and whole)."""
        while True:
            first, second = self._read(2)
            size = second & 0x7F
            if size == 126:
                size = int.from_bytes(self._read(2))
            elif size == 127:
                size = int.from_bytes(self._read(8))
            payload = self._read(size)
            opcode = first & 0x0F
            if opcode == 0x8:
                raise OSError("the joiner closed the socket")
            if opcode == 0x1:
                return cast(dict[str, object], json.loads(payload))

    def send(self, message: dict[str, object]) -> None:
        payload = json.dumps(message).encode()
        mask = os.urandom(4)
        size = len(payload)
        if size < 126:
            head = bytes([0x81, 0x80 | size])
        elif size < 1 << 16:
            head = bytes([0x81, 0x80 | 126]) + size.to_bytes(2)
        else:
            head = bytes([0x81, 0x80 | 127]) + size.to_bytes(8)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.sock.sendall(head + mask + masked)

    def request(self, message: dict[str, object]) -> dict[str, object]:
        """Sends a request (numbered 1) and returns its reply."""
        self.send({**message, "n": 1})
        while True:
            reply = self.next()
            if str(reply["type"]).endswith("_reply") and reply["to"] == 1:
                return reply

    def close(self) -> None:
        self.sock.close()


def hello(joined: Joined) -> dict[str, object]:
    """What a page connecting now is told first: the joiner's state."""
    page = PageSocket(joined)
    try:
        first = page.next()
        assert first["type"] == "hello", first
        return first
    finally:
        page.close()


class FixtureHandler(BaseHTTPRequestHandler):
    """A dev server: the fixture page and a WebSocket echo."""

    protocol_version = "HTTP/1.1"

    def log_message(self, format: str, *_args: object) -> None:
        del format

    def do_GET(self) -> None:
        if self.path == "/socket":
            self.handle_websocket()
            return
        page = urlsplit(self.path).path == "/"
        status, body = (200, FIXTURE_HTML) if page else (404, b"missing")
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        _ = self.wfile.write(body)

    def handle_websocket(self) -> None:
        key = self.headers["Sec-WebSocket-Key"]
        accept = base64.b64encode(
            hashlib.sha1(
                (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()
            ).digest()
        ).decode()
        self.send_response(101)
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()
        self.wfile.flush()
        header = self.rfile.read(2)
        if len(header) != 2:
            return
        length = header[1] & 0x7F
        mask = self.rfile.read(4)
        encoded = self.rfile.read(length)
        payload = bytes(value ^ mask[index % 4] for index, value in enumerate(encoded))
        _ = self.wfile.write(bytes([0x81, len(payload)]) + payload)
        self.wfile.flush()
        self.close_connection = True


@contextmanager
def upstream_server(
    host: str = "127.0.0.1", handler: type[BaseHTTPRequestHandler] = FixtureHandler
) -> Iterator[str]:
    server = ThreadingHTTPServer((host, 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://{host}:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def comment_on_intro(page: Page, text: str) -> None:
    page.locator("#intro").click(button="right")
    textarea = page.locator("#jamparty textarea")
    expect(textarea).to_be_focused()
    textarea.fill(text)
    textarea.press("Enter")
    # Closed once the joiner answered.
    expect(page.locator("#jamparty #jamparty-dialog")).not_to_be_visible()


def recv_all(home: Path) -> list[dict[str, object]]:
    """Every comment the share holds, as `jamparty recv` prints them."""
    return [
        cast(dict[str, object], json.loads(line))
        for line in jamparty(home, "recv").splitlines()
    ]


@contextmanager
def pages(browser: Browser, *joined: Joined) -> Iterator[list[Page]]:
    """One browser context (its own storage) and page per joiner, opened."""
    contexts = [
        browser.new_context(viewport={"width": 1280, "height": 720}) for _ in joined
    ]
    try:
        opened: list[Page] = []
        for context, one in zip(contexts, joined, strict=True):
            page = context.new_page()
            _ = page.goto(one.url)
            opened.append(page)
        yield opened
    finally:
        for context in contexts:
            context.close()


class SocketTap:
    """Routes a page's sockets to its joiner through the test, so messages
    from the joiner of the types in `hold` can be held back, and others
    changed on their way by `edit`."""

    def __init__(self, page: Page) -> None:
        self.hold: set[str] = set()
        self.held: list[tuple[WebSocketRoute, str]] = []
        self.sockets: list[WebSocketRoute] = []
        self.edit: Callable[[dict[str, object]], dict[str, object]] | None = None
        page.route_web_socket("**/_jamparty/api/socket", self._route)

    def _route(self, route: WebSocketRoute) -> None:
        self.sockets.append(route)
        server = route.connect_to_server()

        def from_joiner(message: str | bytes) -> None:
            parsed = cast(dict[str, object], json.loads(message))
            if parsed["type"] in self.hold:
                self.held.append((route, str(message)))
            elif self.edit is not None:
                route.send(json.dumps(self.edit(parsed)))
            else:
                route.send(message)

        server.on_message(from_joiner)
        route.on_message(server.send)

    def release(self) -> None:
        self.hold.clear()
        for route, message in self.held:
            route.send(message)
        self.held.clear()


def overlay(page: Page, selector: str) -> Locator:
    return page.locator(f"#jamparty {selector}")


def switch(page: Page, name: str) -> Locator:
    return overlay(page, f"#{name}")


def settings_path(home: Path) -> Path:
    """The settings file of `home`: read by jamparty, never written."""
    return home / "config/jamparty/settings.json"


def eventually(what: str, check: Callable[[], bool]) -> None:
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(0.05)
    pytest.fail(f"timed out waiting for {what}")


def cursors(page: Page) -> Locator:
    return overlay(page, ".cursor")


def cursor_on(page: Page) -> None:
    """Turns on "Show my cursor" by its key, and waits a little: the others
    hear it is on (presence goes out every 100 ms) before it moves."""
    page.locator("h1").click()
    page.keyboard.press("x")
    expect(overlay(page, "#show-cursor")).to_have_attribute("aria-checked", "true")
    page.wait_for_timeout(300)


def waiting(page: Page, what: str, check: Callable[[], bool]) -> None:
    """Like `eventually`, letting `page` handle its events meanwhile."""
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        if check():
            return
        page.wait_for_timeout(50)
    pytest.fail(f"timed out waiting for {what}")


RECORD_SENT = """() => {
    window.jampartySent = [];
    const original = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
        window.jampartySent.push(JSON.parse(data));
        return original.call(this, data);
    };
}"""


def sent(page: Page, kind: str) -> list[dict[str, object]]:
    """The messages of type `kind` `page` sent its joiner since
    `RECORD_SENT`."""
    messages = cast(list[dict[str, object]], page.evaluate("window.jampartySent"))
    return [message for message in messages if message["type"] == kind]
