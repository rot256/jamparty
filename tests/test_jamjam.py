"""End-to-end tests: real `jamjam serve`/`proxy` and `jamjam join` processes.

Shares run with `--local` (no relays or address lookup) and joiners dial
them with `--share-addr`, so nothing leaves the machine.
"""

import base64
import hashlib
import http.client
import json
import os
import queue
import subprocess
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

import pytest
from playwright.sync_api import Browser, Page, expect, sync_playwright

ROOT = Path(__file__).resolve().parent.parent
TARGET_DIR = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
BINARY = TARGET_DIR / "debug/jamjam"
SITE = ROOT / "tests/fixtures/site"
FIXTURE_HTML = (SITE / "index.html").read_bytes()
TIMEOUT = 20.0


class FixtureHandler(BaseHTTPRequestHandler):
    """A dev server: the fixture page and a WebSocket echo."""

    protocol_version = "HTTP/1.1"

    def log_message(self, format: str, *_args: object) -> None:
        del format

    def do_GET(self) -> None:
        if self.path == "/socket":
            self.handle_websocket()
            return
        status, body = (200, FIXTURE_HTML) if self.path == "/" else (404, b"missing")
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
def upstream_server() -> Iterator[str]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


class Process:
    """A jamjam process whose output lines are collected as they arrive."""

    def __init__(self, arguments: list[str], env: dict[str, str]) -> None:
        self.process: subprocess.Popen[str] = subprocess.Popen(
            [str(BINARY), *arguments],
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
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


@dataclass(frozen=True)
class Joined:
    url: str
    host: str
    port: int


def environment(home: Path) -> dict[str, str]:
    return {**os.environ, "HOME": str(home), "XDG_CONFIG_HOME": str(home / "config")}


@contextmanager
def share(home: Path, *source: str) -> Iterator[Shared]:
    process = Process([*source, "--local"], environment(home))
    try:
        token = process.wait_for("jamjam: token: ")
        address = process.wait_for("jamjam: direct address: ")
        yield Shared(token, address)
    finally:
        process.stop()


@contextmanager
def join_process(home: Path, shared: Shared, token: str) -> Iterator[Process]:
    arguments = ["join", token, "--no-open", "--name", "alice"]
    process = Process([*arguments, "--share-addr", shared.address], environment(home))
    try:
        yield process
    finally:
        process.stop()


@contextmanager
def join(home: Path, shared: Shared) -> Iterator[Joined]:
    with join_process(home, shared, shared.token) as process:
        url = process.wait_for("jamjam: open ")
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
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        try:
            status, body = request(joined, "/_jamjam/api/status")
        except OSError:
            status, body = 0, b""
        if status == 200 and cast(dict[str, object], json.loads(body))["connected"]:
            return
        time.sleep(0.05)
    pytest.fail("the joiner was not admitted")


@pytest.fixture(scope="session", autouse=True)
def build_binary() -> None:
    _ = subprocess.run(["cargo", "build"], cwd=ROOT, check=True)


@pytest.fixture(scope="session", params=["chromium", "firefox"])
def browser(request: pytest.FixtureRequest) -> Iterator[Browser]:
    with sync_playwright() as playwright:
        name = cast(str, request.param)
        browser = getattr(playwright, name).launch()
        assert isinstance(browser, Browser)
        yield browser
        browser.close()


@pytest.fixture()
def page(browser: Browser) -> Iterator[Page]:
    page = browser.new_page(viewport={"width": 1280, "height": 720})
    yield page
    page.close()


def test_serve_and_join_over_http(tmp_path: Path) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        status, body = request(joined, "/")
        assert status == 200
        assert b"Fixture page" in body
        assert b"/_jamjam/client.js?live=0" in body
        status, body = request(joined, "/_jamjam/client.js")
        assert status == 200 and b"api/events" in body
        assert request(joined, "/missing")[0] == 404

        # Wrong Host (DNS rebinding) and cross-site reads are refused.
        assert (
            request(joined, "/", headers={"Host": f"evil.test:{joined.port}"})[0] == 421
        )
        cross = {"Sec-Fetch-Site": "cross-site", "Sec-Fetch-Mode": "cors"}
        assert request(joined, "/", headers=cross)[0] == 403
        comment = json.dumps({"page": "/", "selector": "h1", "comment": "Hi"}).encode()
        posted = request(
            joined,
            "/_jamjam/api/comments",
            method="POST",
            headers={"Content-Type": "application/json", "Origin": "http://evil.test"},
            body=comment,
        )
        assert posted[0] == 403
        status, body = request(
            joined,
            "/_jamjam/api/comments",
            method="POST",
            headers={
                "Content-Type": "application/json",
                "Origin": f"http://{joined.host}",
            },
            body=comment,
        )
        assert status == 202, body
        assert cast(dict[str, object], json.loads(body))["pending"] == 1


def test_a_wrong_token_is_refused(tmp_path: Path) -> None:
    with share(tmp_path, "serve", str(SITE)) as shared:
        key, _secret = shared.token.split(":")
        wrong = f"{key}:{'a' * 26}"
        with join_process(tmp_path, shared, wrong) as process:
            _ = process.wait_for("jamjam: open ")
            output = process.wait_for("jamjam: the share refused")
            assert "token" in output
            assert process.process.wait(timeout=TIMEOUT) == 1


def test_proxy_through_join_with_websockets(tmp_path: Path, page: Page) -> None:
    with (
        upstream_server() as upstream,
        share(tmp_path, "proxy", upstream) as shared,
        join(tmp_path, shared) as joined,
    ):
        status, body = request(joined, "/")
        assert status == 200
        assert b"/_jamjam/client.js" in body
        _ = page.goto(joined.url)
        echoed = page.evaluate(
            """() => new Promise((resolve, reject) => {
                const socket = new WebSocket(`ws://${location.host}/socket`);
                socket.onopen = () => socket.send("hello over iroh");
                socket.onmessage = (event) => resolve(event.data);
                socket.onerror = () => reject(new Error("websocket failed"));
            })"""
        )
        assert echoed == "hello over iroh"


def test_right_click_comment_is_queued(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        assert urlsplit(joined.url).hostname is not None
        assert cast(str, urlsplit(joined.url).hostname).endswith(".localhost")
        _ = page.goto(joined.url)
        badge = page.locator("#jamjam #badge")
        expect(badge).to_have_text("0 queued")
        page.locator("#intro").click(button="right")
        textarea = page.locator("#jamjam textarea")
        expect(textarea).to_be_focused()
        textarea.fill("Tighten this paragraph.")
        textarea.press("Enter")
        expect(badge).to_have_text("1 queued")
        expect(page.locator("#jamjam #toast")).to_contain_text("Comment queued")
