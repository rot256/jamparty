"""End-to-end tests: real `nabe serve`/`proxy` and `nabe join` processes.

Shares run with `--local` (no relays or address lookup) and joiners dial
them with `--share-addr`, so nothing leaves the machine.
"""

import base64
import hashlib
import http.client
import json
import os
import queue
import shutil
import subprocess
import tempfile
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
BINARY = TARGET_DIR / "debug/nabe"
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
    """A nabe process whose output lines are collected as they arrive."""

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


@pytest.fixture(autouse=True)
def remove_runtime_dir(tmp_path: Path) -> Iterator[None]:
    """Removes the test's runtime directory, which lives outside tmp_path."""
    yield
    shutil.rmtree(runtime_dir(tmp_path), ignore_errors=True)


def nabe(home: Path, *arguments: str) -> str:
    """Runs a short nabe command and returns its output."""
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
    process = Process([*source, "--local"], environment(home))
    try:
        token = process.wait_for("nabe: token: ")
        address = process.wait_for("nabe: direct address: ")
        yield Shared(token, address, process)
    finally:
        process.stop()


@contextmanager
def join_process(
    home: Path, shared: Shared, token: str, name: str = "alice"
) -> Iterator[Process]:
    # The token goes on stdin, as `nabe join -` reads it: out of argv.
    arguments = ["join", "-", "--no-open", "--name", name]
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
def join(home: Path, shared: Shared, name: str = "alice") -> Iterator[Joined]:
    with join_process(home, shared, shared.token, name) as process:
        url = process.wait_for("nabe: open ")
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
            status, body = request(joined, "/_nabe/api/status")
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
        assert b"/_nabe/client.js?live=0" in body
        status, body = request(joined, "/_nabe/client.js")
        assert status == 200 and b"api/events" in body
        assert request(joined, "/missing")[0] == 404

        # Wrong Host (DNS rebinding) and cross-site reads are refused.
        assert (
            request(joined, "/", headers={"Host": f"evil.test:{joined.port}"})[0] == 421
        )
        cross = {"Sec-Fetch-Site": "cross-site", "Sec-Fetch-Mode": "cors"}
        assert request(joined, "/", headers=cross)[0] == 403
        comment = json.dumps(
            {"ref": "test-1", "page": "/", "selector": "h1", "comment": "Hi"}
        ).encode()
        posted = request(
            joined,
            "/_nabe/api/comments",
            method="POST",
            headers={"Content-Type": "application/json", "Origin": "http://evil.test"},
            body=comment,
        )
        assert posted[0] == 403
        status, body = request(
            joined,
            "/_nabe/api/comments",
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
            _ = process.wait_for("nabe: open ")
            output = process.wait_for("nabe: the share refused")
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
        assert b"/_nabe/client.js" in body
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
        badge = page.locator("#nabe #badge")
        expect(badge).to_have_text("0 queued")
        page.locator("#intro").click(button="right")
        textarea = page.locator("#nabe textarea")
        expect(textarea).to_be_focused()
        textarea.fill("Tighten this paragraph.")
        textarea.press("Enter")
        expect(badge).to_have_text("1 queued")
        expect(page.locator("#nabe #toast")).to_contain_text("Comment queued")


def test_bare_nabe_and_the_agent_guides(tmp_path: Path) -> None:
    design = (ROOT / "DESIGN.md").read_text()
    block = design.split("Bare `nabe` prints, always the same:", 1)[1]
    overview = block.split("```\n", 2)[1]
    assert nabe(tmp_path) == overview
    guides = nabe(tmp_path, "agent")
    for kind in ["t3", "claude", "codex", "other"]:
        assert f"nabe agent {kind}" in guides
    assert "nabe recv" in nabe(tmp_path, "agent", "claude")
    assert "--t3-url" in nabe(tmp_path, "agent", "t3")
    help_text = nabe(tmp_path, "--help")
    assert "serve" in help_text and "recv" not in help_text


def comment_on_intro(page: Page, text: str) -> None:
    page.locator("#intro").click(button="right")
    textarea = page.locator("#nabe textarea")
    expect(textarea).to_be_focused()
    textarea.fill(text)
    textarea.press("Enter")


def test_delivery_and_answers_in_two_pages(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared, "alice") as alice,
        join(tmp_path, shared, "bob") as bob,
    ):
        alice_page = browser.new_page()
        bob_page = browser.new_page()
        try:
            _ = alice_page.goto(alice.url)
            _ = bob_page.goto(bob.url)
            comment_on_intro(alice_page, "Shorter, please.")
            entry = alice_page.locator("#nabe #outbox li")
            expect(entry).to_have_attribute("data-state", "queued")

            output = nabe(tmp_path, "recv")
            [line] = output.splitlines()
            record = cast(dict[str, object], json.loads(line))
            assert record["comment"] == "Shorter, please."
            assert record["selector"] == "#intro"
            expect(entry).to_have_attribute("data-state", "delivered")
            expect(alice_page.locator("#nabe #badge")).to_have_text("0 queued")

            _ = nabe(
                tmp_path,
                "send",
                "Done: **shorter** now.",
                "--anchor",
                str(record["id"]),
            )
            for page in [alice_page, bob_page]:
                answer = page.locator("#nabe .agent-message")
                expect(answer).to_contain_text("Done: shorter now.")
                expect(answer.locator("strong")).to_have_text("shorter")
        finally:
            alice_page.close()
            bob_page.close()


def test_connection_indicator_and_unsent_comments(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        indicator = page.locator("#nabe #connection")
        expect(indicator).to_have_attribute("data-state", "green")
        shared.process.stop()
        expect(indicator).to_have_attribute("data-state", "grey")
        expect(page.locator("#nabe #connection-text")).to_contain_text("unreachable")

        comment_on_intro(page, "Kept for later.")
        entry = page.locator("#nabe #outbox li")
        expect(entry).to_have_attribute("data-state", "unsent")
        expect(entry.locator("button")).to_have_text("send again")


def test_live_reload_after_a_file_change(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "site"
    _ = shutil.copytree(SITE, site)
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(page.locator("h1")).to_have_text("Fixture page")
        expect(page.locator("#nabe #connection")).to_have_attribute(
            "data-state", "green"
        )
        index = site / "index.html"
        _ = index.write_text(index.read_text().replace("Fixture page", "Changed page"))
        expect(page.locator("h1")).to_have_text("Changed page")
