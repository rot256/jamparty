"""End-to-end tests: real `jampot serve`/`proxy` and `jampot join` processes.

Shares run with `--local` (no relays or address lookup) and joiners dial
them with `--share-addr`, so nothing leaves the machine.
"""

import base64
import hashlib
import http.client
import json
import os
import queue
import re
import shutil
import signal
import socket
import subprocess
import tempfile
import threading
import time
import tomllib
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

import pytest
from playwright.sync_api import (
    APIResponse,
    Browser,
    Locator,
    Page,
    Request,
    Route,
    expect,
    sync_playwright,
)

ROOT = Path(__file__).resolve().parent.parent
TARGET_DIR = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
BINARY = TARGET_DIR / "debug/jampot"
SITE = ROOT / "tests/fixtures/site"
SPA = ROOT / "tests/fixtures/spa"
FIXTURE_HTML = (SITE / "index.html").read_bytes()
ANSI = re.compile(r"\x1b\[[0-9;]*m")
TIMEOUT = 20.0
VERSION = cast(
    str, tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
)


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
    """A jampot process whose output lines are collected as they arrive."""

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


@pytest.fixture(autouse=True)
def remove_runtime_dir(tmp_path: Path) -> Iterator[None]:
    """Removes the test's runtime directory, which lives outside tmp_path."""
    yield
    shutil.rmtree(runtime_dir(tmp_path), ignore_errors=True)


def jampot(home: Path, *arguments: str) -> str:
    """Runs a short jampot command and returns its output."""
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
def share(home: Path, *source: str, log: bool = False) -> Iterator[Shared]:
    """A share; with `log`, its debug log (each cursor message) is kept."""
    env = environment(home)
    if log:
        env["RUST_LOG"] = "jampot=debug"
    process = Process([*source, "--local"], env)
    try:
        token = process.wait_for("jampot: token: ")
        address = process.wait_for("jampot: direct address: ")
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
    # The token goes on stdin, as `jampot join -` reads it: out of argv.
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
        url = process.wait_for("jampot: open ")
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
            status, body = request(joined, "/_jampot/api/status")
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
        assert b"/_jampot/client.js?live=0" in body
        status, body = request(joined, "/_jampot/client.js")
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
            "/_jampot/api/comments",
            method="POST",
            headers={"Content-Type": "application/json", "Origin": "http://evil.test"},
            body=comment,
        )
        assert posted[0] == 403
        status, body = request(
            joined,
            "/_jampot/api/comments",
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
            _ = process.wait_for("jampot: open ")
            output = process.wait_for("jampot: the share refused")
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
        assert b"/_jampot/client.js" in body
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
        badge = page.locator("#jampot #badge")
        expect(badge).to_have_text("0 queued")
        page.locator("#intro").click(button="right")
        textarea = page.locator("#jampot textarea")
        expect(textarea).to_be_focused()
        textarea.fill("Tighten this paragraph.")
        textarea.press("Enter")
        expect(badge).to_have_text("1 queued")
        expect(page.locator("#jampot #toast")).to_contain_text("Comment queued")


def test_bare_jampot_and_the_agent_guides(tmp_path: Path) -> None:
    design = (ROOT / "DESIGN.md").read_text()
    block = design.split("Bare `jampot` prints, always the same:", 1)[1]
    overview = block.split("```\n", 2)[1]
    assert jampot(tmp_path) == overview
    guides = jampot(tmp_path, "agent")
    for kind in ["t3", "claude", "codex", "other"]:
        assert f"jampot agent {kind}" in guides
    assert "jampot recv" in jampot(tmp_path, "agent", "claude")
    assert "--t3-url" in jampot(tmp_path, "agent", "t3")
    help_text = jampot(tmp_path, "--help")
    assert "serve" in help_text and "recv" not in help_text


def comment_on_intro(page: Page, text: str) -> None:
    page.locator("#intro").click(button="right")
    textarea = page.locator("#jampot textarea")
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
            entry = alice_page.locator("#jampot #outbox-list li")
            expect(entry).to_have_attribute("data-state", "queued")

            output = jampot(tmp_path, "recv")
            [line] = output.splitlines()
            record = cast(dict[str, object], json.loads(line))
            assert record["comment"] == "Shorter, please."
            assert record["selector"] == "#intro"
            expect(entry).to_have_attribute("data-state", "delivered")
            expect(alice_page.locator("#jampot #badge")).to_have_text("0 queued")

            _ = jampot(
                tmp_path,
                "send",
                "Done: **shorter** now.",
                "--anchor",
                str(record["id"]),
                "--to",
                "all",
            )
            for page in [alice_page, bob_page]:
                answer = page.locator("#jampot .agent-message")
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
        indicator = page.locator("#jampot #connection")
        expect(indicator).to_have_attribute("data-state", "green")
        shared.process.stop()
        expect(indicator).to_have_attribute("data-state", "grey")
        expect(page.locator("#jampot #connection-text")).to_contain_text("unreachable")

        comment_on_intro(page, "Kept for later.")
        entry = page.locator("#jampot #outbox-list li")
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
        expect(page.locator("#jampot #connection")).to_have_attribute(
            "data-state", "green"
        )
        index = site / "index.html"
        _ = index.write_text(index.read_text().replace("Fixture page", "Changed page"))
        expect(page.locator("h1")).to_have_text("Changed page")


def test_a_joiner_with_another_page_api_is_not_used(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        status = {"api": 999, "version": "9.9.9", "connected": True}
        page.route(
            "**/_jampot/api/status",
            lambda route: route.fulfill(
                status=200, content_type="application/json", body=json.dumps(status)
            ),
        )
        requested: list[str] = []
        page.on("request", lambda request: requested.append(request.url))
        _ = page.goto(joined.url)
        text = page.locator("#jampot #notice")
        expect(text).to_contain_text("your jampot join runs 9.9.9 (page API 999)")
        expect(text).to_contain_text(f"the share runs {VERSION}")
        expect(page.locator("#jampot #connection")).to_have_attribute(
            "data-state", "grey"
        )
        page.wait_for_timeout(500)
        assert not any("/_jampot/api/events" in url for url in requested), requested


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


def test_addressing_by_key(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "alice2", shared, "alice") as alice2,
    ):
        status = cast(
            dict[str, dict[str, str]],
            json.loads(request(alice, "/_jampot/api/status")[1]),
        )
        fingerprint = status["you"]["fingerprint"]
        # Mallory names herself after alice's fingerprint.
        with (
            join(tmp_path / "mallory", shared, fingerprint) as mallory,
            pages(browser, alice, alice2, mallory) as (
                alice_page,
                alice2_page,
                mallory_page,
            ),
        ):
            ambiguous = subprocess.run(
                [str(BINARY), "send", "hi", "--to", "alice"],
                env=environment(tmp_path),
                capture_output=True,
                text=True,
                timeout=TIMEOUT,
                check=False,
            )
            assert ambiguous.returncode == 1
            assert "several people here are called alice" in ambiguous.stderr
            assert f"alice (key:{fingerprint})" in ambiguous.stderr
            _ = jampot(tmp_path, "send", "Only alice.", "--to", f"key:{fingerprint}")
            _ = jampot(tmp_path, "send", "Everyone.")
            for page in (alice2_page, mallory_page):
                messages = page.locator("#jampot .agent-message")
                expect(messages.filter(has_text="Everyone.")).to_have_count(1)
                expect(messages.filter(has_text="Only alice.")).to_have_count(0)
            only = alice_page.locator("#jampot .agent-message", has_text="Only alice.")
            expect(only.locator(".agent-to")).to_be_visible()


def test_addressed_replies(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        comment_on_intro(bob_page, "Is this right?")
        expect(bob_page.locator("#jampot #badge")).to_have_text("1 queued")
        [line] = jampot(tmp_path, "recv").splitlines()
        record = cast(dict[str, object], json.loads(line))

        _ = jampot(tmp_path, "send", "Hello alice", "--to", "alice")
        _ = jampot(tmp_path, "send", "Yes, it is.", "--anchor", str(record["id"]))
        _ = jampot(tmp_path, "send", "Everyone: done.")

        def answers(page: Page) -> list[tuple[str, bool]]:
            messages = page.locator("#jampot .agent-message")
            # Answers arrive in order: the last one means all are here.
            expect(messages.filter(has_text="Everyone: done.")).to_have_count(1)
            return sorted(
                (
                    message.locator(".agent-body").inner_text().strip(),
                    message.locator(".agent-to").is_visible(),
                )
                for message in messages.all()
            )

        assert answers(alice_page) == [
            ("Everyone: done.", False),
            ("Hello alice", True),
        ]
        assert answers(bob_page) == [("Everyone: done.", False), ("Yes, it is.", True)]
        to_you = alice_page.locator("#jampot .agent-to").first
        expect(to_you).to_have_text("to you")
        expect(to_you).to_have_css("color", "rgb(123, 50, 33)")
        expect(to_you).to_have_css("font-size", "11px")
        expect(to_you).to_have_css("font-weight", "700")

        # An ambiguous or unknown name is refused, with the candidates.
        refused = subprocess.run(
            [str(BINARY), "send", "hi", "--to", "zed"],
            env=environment(tmp_path),
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
        assert refused.returncode == 1
        assert "nobody called zed is here" in refused.stderr
        assert "alice (" in refused.stderr and "bob (" in refused.stderr


# The overlay (docs/ui.md), one joiner at a time.


def overlay(page: Page, selector: str) -> Locator:
    return page.locator(f"#jampot {selector}")


def test_the_pill_holds_the_dot_and_follows_the_state(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join_process(tmp_path, shared, shared.token) as process,
    ):
        url = process.wait_for("jampot: open ")
        context = browser.new_context()
        try:
            page = context.new_page()
            page.clock.install()
            _ = page.goto(url)
            dot = overlay(page, "#pill > #connection")
            expect(dot).to_have_attribute("data-state", "green")
            expect(dot).to_have_css("background-color", "rgb(47, 158, 68)")
            expect(overlay(page, "#pill")).to_contain_text("0 queued")
            expect(overlay(page, "#bar > #gear")).to_be_visible()

            # The joiner's event stream goes silent: the page's watchdog
            # gives up on it and reconnects, which cannot work yet.
            os.kill(process.process.pid, signal.SIGSTOP)
            try:
                page.clock.fast_forward(40_000)
                expect(dot).to_have_attribute("data-state", "amber")
                expect(dot).to_have_css("background-color", "rgb(232, 163, 61)")
            finally:
                os.kill(process.process.pid, signal.SIGCONT)
            expect(dot).to_have_attribute("data-state", "green")

            shared.process.stop()
            expect(dot).to_have_attribute("data-state", "grey")
            expect(dot).to_have_css("background-color", "rgb(154, 149, 142)")
        finally:
            context.close()


def test_the_comment_list(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        comment_on_intro(page, "First, on the paragraph.")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        _ = page.keyboard.press("/")
        textarea = overlay(page, "textarea")
        textarea.fill("Second, on the page.")
        textarea.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("2 queued")

        # Expand: the pill becomes the list, newest first.
        overlay(page, "#pill").click()
        listed = overlay(page, "#list")
        expect(listed).to_be_visible()
        expect(overlay(page, "#bar")).to_be_hidden()
        expect(overlay(page, "#list-count")).to_have_text("2 queued")
        expect(overlay(page, "#connection-text")).to_have_text("· connected")
        rows = overlay(page, "#outbox-list li")
        expect(rows).to_have_count(2)
        expect(rows.nth(0).locator(".where")).to_have_text("page · just now")
        expect(rows.nth(0).locator(".text")).to_have_text("Second, on the page.")
        expect(rows.nth(1).locator(".where")).to_have_text("#intro · just now")
        expect(rows.nth(1).locator(".state")).to_have_text("queued")

        # Delivered once the agent has them, in green.
        assert len(jampot(tmp_path, "recv").splitlines()) == 2
        expect(rows.nth(1).locator(".state")).to_have_text("delivered")
        expect(rows.nth(1).locator(".state")).to_have_css("color", "rgb(140, 233, 154)")
        expect(overlay(page, "#list-count")).to_have_text("0 queued")

        # Collapse with ▾, and by clicking the header.
        overlay(page, "#collapse").click()
        expect(listed).to_be_hidden()
        expect(overlay(page, "#pill")).to_be_visible()
        overlay(page, "#pill").click()
        overlay(page, "#list-summary").click()
        expect(listed).to_be_hidden()

        # Not sent (the joiner did not take it), then sent again.
        page.route("**/_jampot/api/comments", lambda route: route.abort())
        comment_on_intro(page, "Third, kept for later.")
        overlay(page, "#pill").click()
        unsent = rows.nth(0)
        expect(unsent).to_have_attribute("data-state", "unsent")
        expect(unsent.locator(".state")).to_have_text("not sent")
        expect(unsent.locator(".state")).to_have_css("color", "rgb(255, 168, 168)")
        page.unroute("**/_jampot/api/comments")
        unsent.locator("button.again").click()
        expect(unsent).to_have_attribute("data-state", "queued")
        expect(overlay(page, "#list-count")).to_have_text("1 queued")


def switch(page: Page, name: str) -> Locator:
    return overlay(page, f"#{name}")


def test_the_control_panel(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        # A text field of the page's own, and a record of what the page saw.
        _ = page.evaluate(
            """() => {
                const field = document.createElement("input");
                field.id = "field";
                document.querySelector("main").append(field);
                window.seen = [];
                for (const type of ["contextmenu", "keydown"]) {
                    window.addEventListener(type, (event) => {
                        window.seen.push(`${type}:${event.defaultPrevented}`);
                    });
                }
            }"""
        )
        comment_mode = switch(page, "comment-mode")
        show_cursor = switch(page, "show-cursor")

        # One panel at a time.
        overlay(page, "#gear").click()
        expect(overlay(page, "#panel")).to_be_visible()
        overlay(page, "#pill").click()
        expect(overlay(page, "#list")).to_be_visible()
        expect(overlay(page, "#panel")).to_be_hidden()
        overlay(page, "#collapse").click()
        overlay(page, "#gear").click()
        expect(overlay(page, "#panel")).to_be_visible()
        expect(overlay(page, "#panel")).to_have_css("width", "232px")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        expect(show_cursor).to_have_attribute("aria-checked", "false")
        expect(overlay(page, "#comment-mode-key")).to_have_text("C")
        expect(overlay(page, "#show-cursor-key")).to_have_text("X")

        # By click.
        comment_mode.click()
        expect(comment_mode).to_have_attribute("aria-checked", "false")
        show_cursor.click()
        expect(show_cursor).to_have_attribute("aria-checked", "true")

        # Comment mode off: right-click and / are the page's.
        _ = page.evaluate("window.seen = []")
        page.locator("#intro").click(button="right")
        page.keyboard.press("Escape")
        page.locator("h1").click()
        page.keyboard.press("/")
        expect(overlay(page, "#jampot-dialog")).not_to_be_visible()
        assert page.evaluate("window.seen") == [
            "contextmenu:false",
            "keydown:false",
            "keydown:false",
        ]

        # By key; not while a text field has focus.
        page.keyboard.press("c")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        page.keyboard.press("x")
        expect(show_cursor).to_have_attribute("aria-checked", "false")
        page.locator("#field").click()
        page.keyboard.type("cx")
        expect(page.locator("#field")).to_have_value("cx")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        expect(show_cursor).to_have_attribute("aria-checked", "false")
        page.locator("h1").click()

        # Rebinding: the new key works, the old one does not.
        chip = overlay(page, "#comment-mode-key")
        chip.click()
        expect(chip).to_have_text("press a key…")
        expect(chip).to_have_css("border-style", "dashed")
        page.keyboard.press("k")
        expect(chip).to_have_text("K")
        page.keyboard.press("c")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        page.keyboard.press("k")
        expect(comment_mode).to_have_attribute("aria-checked", "false")
        # Escape cancels.
        chip.click()
        page.keyboard.press("Escape")
        expect(chip).to_have_text("K")
        # Keys taken by the other switch or by jampot are refused.
        chip.click()
        page.keyboard.press("x")
        expect(overlay(page, "#panel-message")).to_have_text(
            "X is taken by show my cursor"
        )
        expect(chip).to_have_text("K")
        chip.click()
        page.keyboard.press("/")
        expect(overlay(page, "#panel-message")).to_have_text("/ is used by jampot")
        expect(chip).to_have_text("K")
        expect(overlay(page, "#jampot-dialog")).not_to_be_visible()

        # Comment mode back on: / is jampot's again.
        page.keyboard.press("k")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        page.keyboard.press("/")
        expect(overlay(page, "textarea")).to_be_focused()


def settings_file(home: Path) -> dict[str, object]:
    path = home / "config/jampot/settings.json"
    assert path.stat().st_mode & 0o777 == 0o600
    return cast(dict[str, object], json.loads(path.read_text()))


def test_the_name_prompt_and_settings_on_disk(tmp_path: Path, browser: Browser) -> None:
    home = tmp_path / "dora"
    context = browser.new_context()
    try:
        with (
            share(tmp_path, "serve", str(SITE)) as shared,
            join(home, shared, name=None) as joined,
        ):
            page = context.new_page()
            _ = page.goto(joined.url)
            prompt = overlay(page, "#name-dialog")
            expect(prompt).to_be_visible()
            expect(prompt).to_contain_text("Join the potluck.")
            expect(prompt).to_contain_text(
                "Remembered on this computer; change it under the gear."
            )
            name = overlay(page, "#name-input")
            expect(name).not_to_have_value("")
            name.fill("dora")
            overlay(page, "#name-join").click()
            expect(prompt).to_be_hidden()
            assert settings_file(home)["name"] == "dora"

            # Under the gear: the name, and who is here.
            overlay(page, "#gear").click()
            expect(overlay(page, "#name-field")).to_have_value("dora")
            expect(overlay(page, "#here-count")).to_have_text("1 here")
            expect(overlay(page, ".avatar")).to_have_attribute("title", "dora (you)")
            # A change updates presence.
            overlay(page, "#name-field").fill("dora.b")
            overlay(page, "#name-field").press("Enter")
            expect(overlay(page, ".avatar")).to_have_attribute("title", "dora.b (you)")
            expect(overlay(page, ".avatar")).to_have_text("DB")
            # Keys and switches are kept too.
            overlay(page, "#show-cursor").click()
            overlay(page, "#comment-mode-key").click()
            page.keyboard.press("k")
            expect(overlay(page, "#comment-mode-key")).to_have_text("K")
            page.close()
            saved = settings_file(home)
            assert saved["name"] == "dora.b"
            assert saved["showCursor"] == "on"
            assert saved["keys"] == {"commentMode": "k", "showCursor": "x"}

        # A new share and a new joiner, the same home: no prompt, the same
        # settings.
        with (
            share(tmp_path, "serve", str(SITE)) as shared,
            join(home, shared, name=None) as joined,
        ):
            page = context.new_page()
            _ = page.goto(joined.url)
            overlay(page, "#gear").click()
            expect(overlay(page, "#name-field")).to_have_value("dora.b")
            expect(overlay(page, "#name-dialog")).to_be_hidden()
            expect(overlay(page, "#show-cursor")).to_have_attribute(
                "aria-checked", "true"
            )
            expect(overlay(page, "#comment-mode-key")).to_have_text("K")
            expect(overlay(page, ".avatar")).to_have_attribute("title", "dora.b (you)")
    finally:
        context.close()


# Several joiners at once: alice, bob and carol, each with their own home
# directory (key and settings), joiner process and browser context.


def eventually(what: str, check: Callable[[], bool]) -> None:
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(0.05)
    pytest.fail(f"timed out waiting for {what}")


def cursors(page: Page) -> Locator:
    return overlay(page, ".cursor")


def cursor_at(page: Page) -> tuple[float, float] | None:
    """Where the one cursor drawn in `page` is, if any."""
    transform = cast(
        str | None,
        page.evaluate(
            """() => document.querySelector("#jampot")
                .shadowRoot.querySelector(".cursor")?.style.transform ?? null"""
        ),
    )
    if not transform:
        return None
    x, y = re.findall(r"-?[\d.]+", transform)[:2]
    return float(x), float(y)


def near(page: Page, x: float, y: float) -> bool:
    at = cursor_at(page)
    return at is not None and abs(at[0] - x) < 2 and abs(at[1] - y) < 2


def cursor_messages(shared: Shared, name: str) -> int:
    """Cursor messages from `name` that reached the share since the last look."""
    return sum(
        "cursor message" in line and f"name={name}" in line
        for line in shared.process.drain()
    )


def waiting(page: Page, what: str, check: Callable[[], bool]) -> None:
    """Like `eventually`, letting `page` handle its events meanwhile."""
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        if check():
            return
        page.wait_for_timeout(50)
    pytest.fail(f"timed out waiting for {what}")


def cursor_posts(page: Page) -> list[dict[str, object]]:
    """The cursor messages `page` posts to its joiner, as they are made."""
    posts: list[dict[str, object]] = []

    def record(request: Request) -> None:
        if request.url.endswith("/_jampot/api/cursor"):
            posts.append(cast(dict[str, object], request.post_data_json))

    page.on("request", record)
    return posts


def test_an_unrepresentable_url_is_nowhere(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SPA), log=True) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        overlay(alice_page, "#gear").click()
        expect(overlay(alice_page, "#here-count")).to_have_text("2 here")
        bob_page.locator("#title").click()
        bob_page.keyboard.press("x")
        expect(overlay(bob_page, "#show-cursor")).to_have_attribute(
            "aria-checked", "true"
        )
        time.sleep(0.3)
        bob_page.mouse.move(300, 250)
        expect(cursors(alice_page)).to_have_count(1)

        # Too long to say: bob is nowhere, and his pointer goes nowhere.
        _ = bob_page.evaluate("history.pushState({}, '', '/?' + 'q'.repeat(600))")
        expect(cursors(alice_page)).to_have_count(0)
        time.sleep(0.5)
        _ = cursor_messages(shared, "bob")
        for step in range(10):
            bob_page.mouse.move(300 + step * 5, 260)
        time.sleep(0.5)
        assert cursor_messages(shared, "bob") == 0
        expect(cursors(alice_page)).to_have_count(0)

        # Back on a page it can name: there again.
        _ = bob_page.evaluate("history.pushState({}, '', '/')")
        time.sleep(0.3)
        bob_page.mouse.move(320, 270)
        expect(cursors(alice_page)).to_have_count(1)


def test_cursor_messages_coalesce_behind_a_slow_one(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SPA)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        overlay(alice_page, "#gear").click()
        expect(overlay(alice_page, "#here-count")).to_have_text("2 here")
        bob_page.locator("#title").click()
        bob_page.keyboard.press("x")
        expect(overlay(bob_page, "#show-cursor")).to_have_attribute(
            "aria-checked", "true"
        )
        time.sleep(0.3)
        held: list[Route] = []
        bob_page.route("**/_jampot/api/cursor", lambda route: held.append(route))
        posts = cursor_posts(bob_page)

        def release() -> None:
            while held:
                held.pop(0).continue_()

        def burst(start: int) -> None:
            for step in range(12):
                bob_page.mouse.move(start + step * 10, 250)
                bob_page.wait_for_timeout(30)

        # One request in flight: the moves behind it wait as one, the last.
        burst(200)
        waiting(bob_page, "the first request", lambda: len(posts) == 1)
        bob_page.wait_for_timeout(500)
        assert len(posts) == 1
        release()
        waiting(bob_page, "the coalesced one", lambda: len(posts) == 2)
        release()
        bob_page.wait_for_timeout(500)
        assert len(posts) == 2, posts
        eventually("bob's last position", lambda: near(alice_page, 310, 250))

        # A leave replaces the positions waiting behind a slow request.
        burst(200)
        waiting(bob_page, "a held request", lambda: len(posts) == 3)
        burst(400)
        _ = bob_page.evaluate("window.dispatchEvent(new Event('blur'))")
        release()
        waiting(bob_page, "the leave", lambda: len(posts) == 4)
        release()
        bob_page.wait_for_timeout(500)
        assert len(posts) == 4, posts
        assert "position" not in posts[3]
        expect(cursors(alice_page)).to_have_count(0)

        # Switched off behind a slow request: nothing more goes out.
        burst(200)
        waiting(bob_page, "a held request", lambda: len(posts) == 5)
        burst(400)
        bob_page.keyboard.press("x")
        expect(overlay(bob_page, "#show-cursor")).to_have_attribute(
            "aria-checked", "false"
        )
        release()
        bob_page.wait_for_timeout(500)
        release()
        assert len(posts) == 5, posts


def cursor_on(page: Page) -> None:
    page.locator("#title").click()
    page.keyboard.press("x")
    expect(overlay(page, "#show-cursor")).to_have_attribute("aria-checked", "true")
    page.wait_for_timeout(300)


def bob_is_at(bob: Joined, url: str) -> Callable[[], bool]:
    def check() -> bool:
        status = cast(
            dict[str, list[dict[str, object]]],
            json.loads(request(bob, "/_jampot/api/status")[1]),
        )
        return any(
            person["name"] == "bob" and person["url"] == url
            for person in status["here"]
        )

    return check


def test_a_burst_of_page_changes_ends_on_the_last(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SPA)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        overlay(alice_page, "#gear").click()
        expect(overlay(alice_page, "#here-count")).to_have_text("2 here")
        cursor_on(bob_page)
        # Straight at the joiner: 65 reports as fast as they go.
        _ = bob_page.evaluate(
            """async () => {
                const stream = new EventSource("/_jampot/api/events?tab=burst&url=/");
                await new Promise((resolve) => { stream.onopen = resolve; });
                for (let seq = 1; seq <= 65; seq++) {
                    await fetch("/_jampot/api/page", {
                        method: "POST",
                        headers: { "Content-Type": "application/json" },
                        body: JSON.stringify({ tab: "burst", seq, url: `/q${seq}` }),
                    });
                }
                window.burst = stream;
            }"""
        )
        eventually("the share at /q65", bob_is_at(bob, "/q65"))
        # And through the page itself: 65 route changes, ending on /r64.
        _ = bob_page.evaluate(
            """() => {
                window.burst.close();
                for (let index = 0; index < 65; index++) {
                    history.pushState({}, "", `/r${index}`);
                }
            }"""
        )
        eventually("the share at /r64", bob_is_at(bob, "/r64"))
        # Cursors go by it: alice on /r64 sees bob there.
        _ = alice_page.evaluate("history.pushState({}, '', '/r64')")
        alice_page.wait_for_timeout(500)
        bob_page.mouse.move(300, 250)
        bob_page.mouse.move(310, 255)
        expect(cursors(alice_page)).to_have_count(1)


def test_page_reports_out_of_order(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SPA)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        _ = alice_page.evaluate("history.pushState({}, '', '/b')")
        cursor_on(bob_page)
        held: list[Route] = []

        def hold_a(route: Route) -> None:
            body = cast(dict[str, object], route.request.post_data_json)
            if body["url"] == "/a":
                held.append(route)
            else:
                route.continue_()

        bob_page.route("**/_jampot/api/page", hold_a)
        _ = bob_page.evaluate("history.pushState({}, '', '/a')")
        waiting(bob_page, "the report of /a", lambda: len(held) > 0)
        _ = bob_page.evaluate("history.pushState({}, '', '/b')")
        bob_page.wait_for_timeout(300)
        held.pop().continue_()
        waiting(bob_page, "the share at /b", bob_is_at(bob, "/b"))
        bob_page.wait_for_timeout(500)
        assert bob_is_at(bob, "/b")()
        bob_page.mouse.move(300, 250)
        bob_page.mouse.move(310, 255)
        expect(cursors(alice_page)).to_have_count(1)


def test_movement_during_a_page_report_is_sent(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SPA)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        _ = alice_page.evaluate("history.pushState({}, '', '/other')")
        cursor_on(bob_page)
        held: list[Route] = []
        bob_page.route("**/_jampot/api/page", lambda route: held.append(route))
        _ = bob_page.evaluate("history.pushState({}, '', '/other')")
        waiting(bob_page, "the report", lambda: len(held) > 0)
        # The pointer moves and stops while the page is being reported.
        bob_page.mouse.move(320, 260)
        bob_page.wait_for_timeout(200)
        for route in held:
            route.continue_()
        expect(cursors(alice_page)).to_have_count(1)
        eventually("the cursor where bob stopped", lambda: near(alice_page, 320, 260))


def test_moving_pages_and_pointers_stay_connected(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SPA), log=True) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        join(tmp_path / "carol", shared, "carol") as carol,
        pages(browser, alice, bob, carol) as (alice_page, bob_page, carol_page),
    ):
        _ = alice_page.evaluate("history.pushState({}, '', '/route419')")
        _ = carol_page.evaluate("history.pushState({}, '', '/route0')")
        cursor_on(bob_page)
        # A new page and a pointer movement every 70 ms, 420 times.
        _ = bob_page.evaluate(
            """async () => {
                for (let index = 0; index < 420; index++) {
                    history.pushState({}, "", `/route${index}`);
                    document.dispatchEvent(new PointerEvent("pointermove", {
                        clientX: 320, clientY: 260, pointerType: "mouse", bubbles: true,
                    }));
                    await new Promise((resolve) => setTimeout(resolve, 70));
                }
            }"""
        )
        eventually("the share at /route419", bob_is_at(bob, "/route419"))
        bob_page.mouse.move(300, 250)
        bob_page.mouse.move(310, 255)
        expect(cursors(alice_page)).to_have_count(1)
        expect(cursors(carol_page)).to_have_count(0)
        log = "\n".join(shared.process.drain() + shared.process.seen)
        assert "presence flood" not in log
        assert "joiner left" not in log


def test_cursors_follow_pointers_on_the_same_page(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SPA), log=True) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        join(tmp_path / "carol", shared, "carol") as carol,
        pages(browser, alice, bob, carol) as (alice_page, bob_page, carol_page),
    ):
        for page in (alice_page, bob_page, carol_page):
            overlay(page, "#gear").click()
            expect(overlay(page, "#here-count")).to_have_text("3 here")
            overlay(page, "#gear").click()
        # Carol moves to another route of the single-page app.
        carol_page.locator("#to-other").click()
        expect(carol_page.locator("#title")).to_have_text("Other")

        # Off by default: bob's pointer goes nowhere.
        _ = cursor_messages(shared, "bob")
        for step in range(10):
            bob_page.mouse.move(200 + step * 10, 250)
        time.sleep(0.5)
        assert cursor_messages(shared, "bob") == 0

        # Bob turns his on: alice sees it, named, following his pointer.
        bob_page.keyboard.press("x")
        expect(overlay(bob_page, "#show-cursor")).to_have_attribute(
            "aria-checked", "true"
        )
        time.sleep(0.3)
        bob_page.mouse.move(300, 250)
        expect(cursors(alice_page)).to_have_count(1)
        expect(cursors(alice_page).locator(".tag")).to_have_text("bob")
        expect(cursors(alice_page).locator(".tag")).to_have_css("font-size", "11px")
        eventually("the cursor at bob's pointer", lambda: near(alice_page, 300, 250))
        bob_page.mouse.move(500, 320)
        eventually("the cursor following", lambda: near(alice_page, 500, 320))
        expect(cursors(carol_page)).to_have_count(0)

        # A cursor is never what a comment is about.
        alice_page.mouse.click(500, 320, button="right")
        textarea = overlay(alice_page, "textarea")
        expect(textarea).to_be_focused()
        textarea.fill("Under bob's cursor.")
        textarea.press("Enter")
        expect(overlay(alice_page, "#badge")).to_have_text("1 queued")
        [line] = jampot(tmp_path, "recv").splitlines()
        assert cast(dict[str, object], json.loads(line))["selector"] == "#board"

        # Held still, nothing is sent.
        time.sleep(0.3)
        _ = cursor_messages(shared, "bob")
        time.sleep(3)
        assert cursor_messages(shared, "bob") == 0
        # Without updates it fades (on the receiver's own timer).
        expect(cursors(alice_page)).to_have_attribute("data-faded", "true")
        expect(cursors(alice_page)).to_have_css("opacity", "0")

        # A burst of movement: at most about 15 per second, ending on the
        # last position.
        started = time.monotonic()
        for step in range(60):
            bob_page.mouse.move(200 + step * 5, 260)
            time.sleep(1 / 60)
        elapsed = time.monotonic() - started
        eventually("the final position", lambda: near(alice_page, 495, 260))
        time.sleep(0.3)
        sent = cursor_messages(shared, "bob")
        assert 3 <= sent <= 15 * elapsed + 2, (sent, elapsed)

        # Leaving the page takes the cursor away at once.
        _ = bob_page.evaluate("window.dispatchEvent(new Event('blur'))")
        expect(cursors(alice_page)).to_have_count(0)
        bob_page.mouse.move(300, 250)
        expect(cursors(alice_page)).to_have_count(1)

        # A route change: bob's cursor leaves alice and reaches carol.
        bob_page.locator("#to-other").click()
        expect(bob_page.locator("#title")).to_have_text("Other")
        expect(cursors(alice_page)).to_have_count(0)
        bob_page.mouse.move(320, 260)
        expect(cursors(carol_page)).to_have_count(1)
        eventually("the cursor for carol", lambda: near(carol_page, 320, 260))
        expect(cursors(alice_page)).to_have_count(0)

        # Over an element only bob's page has: not drawn for carol.
        _ = bob_page.evaluate(
            """() => {
                const only = document.createElement("div");
                only.id = "only-bob";
                only.style.cssText =
                    "position:fixed;left:0;top:0;width:200px;height:120px;background:#fcc";
                document.body.append(only);
            }"""
        )
        bob_page.mouse.move(50, 50)
        expect(cursors(carol_page)).to_have_count(0)
        bob_page.mouse.move(320, 260)
        expect(cursors(carol_page)).to_have_count(1)

        # Bob turns it off: it disappears, and the share hears no more.
        bob_page.keyboard.press("x")
        expect(cursors(carol_page)).to_have_count(0)
        time.sleep(0.3)
        _ = cursor_messages(shared, "bob")
        for step in range(30):
            bob_page.mouse.move(250 + step * 5, 280)
        time.sleep(1)
        assert cursor_messages(shared, "bob") == 0
        expect(cursors(alice_page)).to_have_count(0)


def avatars(page: Page) -> dict[str, str]:
    """Each avatar's name (its title) and colour."""
    return cast(
        dict[str, str],
        page.evaluate(
            """() => Object.fromEntries(
                [...document.querySelector("#jampot").shadowRoot
                    .querySelectorAll(".avatar")]
                .map((avatar) => [avatar.title, getComputedStyle(avatar).backgroundColor]))"""
        ),
    )


def test_presence_in_every_page(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        for page in (alice_page, bob_page):
            overlay(page, "#gear").click()
            expect(overlay(page, "#here-count")).to_have_text("2 here")
        with (
            join(tmp_path / "carol", shared, "carol") as carol,
            pages(browser, carol) as (carol_page,),
        ):
            overlay(carol_page, "#gear").click()
            everyone = (alice_page, bob_page, carol_page)
            for page in everyone:
                expect(overlay(page, "#here-count")).to_have_text("3 here")
                expect(overlay(page, ".avatar")).to_have_count(3)
            # The joiner's own avatar is outlined, the others filled in
            # their colour, the same colour in every page.
            expect(overlay(alice_page, '.avatar[data-you="true"]')).to_have_attribute(
                "title", "alice (you)"
            )
            expect(overlay(alice_page, '.avatar[data-you="true"]')).to_have_css(
                "border-top-color", "rgb(255, 255, 255)"
            )
            bob_colour = avatars(alice_page)["bob"]
            assert bob_colour == avatars(carol_page)["bob"] != "rgb(22, 22, 22)"
            assert avatars(alice_page)["carol"] == avatars(bob_page)["carol"]

            # A rename reaches everyone.
            overlay(bob_page, "#name-field").fill("robert")
            overlay(bob_page, "#name-field").press("Enter")
            for page in (alice_page, carol_page):
                expect(overlay(page, '.avatar[title="robert"]')).to_have_count(1)
                expect(overlay(page, '.avatar[title="bob"]')).to_have_count(0)
            assert avatars(carol_page)["robert"] == bob_colour
        # Carol leaves.
        for page in (alice_page, bob_page):
            expect(overlay(page, "#here-count")).to_have_text("2 here")
            expect(overlay(page, '.avatar[title="carol"]')).to_have_count(0)


def test_comments_stay_with_their_author(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        # What each page's event stream carries, as the page would see it.
        for page in (alice_page, bob_page):
            _ = page.evaluate(
                """() => {
                    window.seen = [];
                    const stream = new EventSource("/_jampot/api/events");
                    stream.onmessage = (event) => window.seen.push(event.data);
                }"""
            )
        comment_on_intro(alice_page, "Alice's comment.")
        expect(overlay(alice_page, "#badge")).to_have_text("1 queued")
        [line] = jampot(tmp_path, "recv").splitlines()
        record = cast(dict[str, object], json.loads(line))
        assert record["comment"] == "Alice's comment."
        _ = jampot(tmp_path, "send", "Marker.", "--to", "all")

        def seen(page: Page) -> list[str]:
            return cast(list[str], page.evaluate("window.seen"))

        # Events arrive in order: once the marker is in, so is any delivery.
        def marked(page: Page) -> Callable[[], bool]:
            return lambda: any("Marker." in event for event in seen(page))

        for page in (alice_page, bob_page):
            waiting(page, "the marker", marked(page))
        assert any(
            '"type":"delivered"' in event and str(record["id"]) in event
            for event in seen(alice_page)
        ), seen(alice_page)
        assert not any('"type":"delivered"' in event for event in seen(bob_page)), seen(
            bob_page
        )
        comment_on_intro(bob_page, "Bob's comment.")
        expect(overlay(bob_page, "#badge")).to_have_text("1 queued")

        alice_rows = overlay(alice_page, "#outbox-list li")
        bob_rows = overlay(bob_page, "#outbox-list li")
        expect(alice_rows).to_have_count(1)
        expect(alice_rows.locator(".text")).to_have_text("Alice's comment.")
        expect(alice_rows).to_have_attribute("data-state", "delivered")
        expect(overlay(alice_page, "#badge")).to_have_text("0 queued")
        # Bob sees only his own, still queued: alice's delivery is not his.
        expect(bob_rows).to_have_count(1)
        expect(bob_rows.locator(".text")).to_have_text("Bob's comment.")
        expect(bob_rows).to_have_attribute("data-state", "queued")
        expect(overlay(bob_page, "#badge")).to_have_text("1 queued")


def free_port() -> int:
    """A port free on 127.0.0.1 and [::1] (as far as can be told)."""
    with socket.socket(socket.AF_INET6) as v6:
        v6.bind(("::1", 0))
        port = cast(int, v6.getsockname()[1])
    with socket.socket(socket.AF_INET) as v4:
        v4.bind(("127.0.0.1", port))
    return port


RECONNECT = 20_000


def test_a_joiner_that_drops_comes_back(tmp_path: Path, browser: Browser) -> None:
    port = free_port()
    bob_context = browser.new_context(viewport={"width": 1280, "height": 720})
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "carol", shared, "carol") as carol,
        pages(browser, alice, carol) as (alice_page, carol_page),
    ):
        try:
            with join(tmp_path / "bob", shared, "bob", port=port) as bob:
                bob_page = bob_context.new_page()
                _ = bob_page.goto(bob.url)
                for page in (alice_page, carol_page):
                    overlay(page, "#gear").click()
                    expect(overlay(page, "#here-count")).to_have_text("3 here")
                bob_page.locator("h1").click()
                bob_page.keyboard.press("x")
                expect(overlay(bob_page, "#show-cursor")).to_have_attribute(
                    "aria-checked", "true"
                )
                time.sleep(0.3)
                bob_page.mouse.move(400, 300)
                expect(cursors(alice_page)).to_have_count(1)

            # Bob's joiner stopped: the others see him go, cursor and all.
            for page in (alice_page, carol_page):
                expect(overlay(page, "#here-count")).to_have_text("2 here")
            expect(cursors(alice_page)).to_have_count(0)
            expect(overlay(bob_page, "#connection")).not_to_have_attribute(
                "data-state", "green"
            )
            comment_on_intro(bob_page, "Written while away.")
            expect(overlay(bob_page, "#outbox-list li")).to_have_attribute(
                "data-state", "unsent"
            )

            # Back on the same port (the same page): the comment goes out.
            with join(tmp_path / "bob", shared, "bob", port=port):
                expect(overlay(bob_page, "#connection")).to_have_attribute(
                    "data-state", "green", timeout=RECONNECT
                )
                expect(overlay(bob_page, "#outbox-list li")).to_have_attribute(
                    "data-state", "queued", timeout=RECONNECT
                )
                [line] = jampot(tmp_path, "recv").splitlines()
                record = cast(dict[str, object], json.loads(line))
                assert record["comment"] == "Written while away."
                for page in (alice_page, carol_page):
                    expect(overlay(page, "#here-count")).to_have_text("3 here")
                bob_page.mouse.move(420, 310)
                bob_page.mouse.move(430, 320)
                expect(cursors(alice_page)).to_have_count(1)
                expect(cursors(alice_page).locator(".tag")).to_have_text("bob")
        finally:
            bob_context.close()


def test_reconnecting_after_a_client_side_move(
    tmp_path: Path, browser: Browser
) -> None:
    port = free_port()
    bob_context = browser.new_context(viewport={"width": 1280, "height": 720})
    with (
        share(tmp_path, "serve", str(SPA)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "carol", shared, "carol") as carol,
        pages(browser, alice, carol) as (alice_page, carol_page),
    ):
        try:
            carol_page.locator("#to-other").click()
            expect(carol_page.locator("#title")).to_have_text("Other")
            with join(tmp_path / "bob", shared, "bob", port=port) as bob:
                bob_page = bob_context.new_page()
                _ = bob_page.goto(bob.url)
                bob_page.locator("#title").click()
                bob_page.keyboard.press("x")
                expect(overlay(bob_page, "#show-cursor")).to_have_attribute(
                    "aria-checked", "true"
                )
                bob_page.locator("#to-other").click()
                expect(bob_page.locator("#title")).to_have_text("Other")
            # The event stream reconnects by itself, to a URL naming "/".
            with join(tmp_path / "bob", shared, "bob", port=port):
                expect(overlay(bob_page, "#connection")).to_have_attribute(
                    "data-state", "green", timeout=RECONNECT
                )
                time.sleep(0.5)
                for step in range(5):
                    bob_page.mouse.move(300 + step * 10, 250)
                    time.sleep(0.1)
                expect(cursors(carol_page)).to_have_count(1)
                time.sleep(0.5)
                expect(cursors(alice_page)).to_have_count(0)
        finally:
            bob_context.close()


# Further cases of docs/ui.md.


def test_keys_rebinding_and_text_fields(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(
            """() => {
                const area = document.createElement("textarea");
                area.id = "area";
                const editable = document.createElement("div");
                editable.id = "editable";
                editable.contentEditable = "true";
                editable.textContent = "edit:";
                document.querySelector("main").append(area, editable);
            }"""
        )
        comment_mode = switch(page, "comment-mode")
        show_cursor = switch(page, "show-cursor")
        for field in ("#area", "#editable"):
            page.locator(field).click()
            page.keyboard.type("cx")
            expect(comment_mode).to_have_attribute("aria-checked", "true")
            expect(show_cursor).to_have_attribute("aria-checked", "false")
        expect(page.locator("#area")).to_have_value("cx")
        expect(page.locator("#editable")).to_contain_text("cx")

        # The cursor's key rebinds too.
        page.locator("h1").click()
        overlay(page, "#gear").click()
        chip = overlay(page, "#show-cursor-key")
        chip.click()
        page.keyboard.press("y")
        expect(chip).to_have_text("Y")
        page.keyboard.press("x")
        expect(show_cursor).to_have_attribute("aria-checked", "false")
        page.keyboard.press("y")
        expect(show_cursor).to_have_attribute("aria-checked", "true")


def test_list_times_and_dialog_titles(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.clock.install()
        _ = page.goto(joined.url)
        label = overlay(page, "#dialog-label")
        page.locator("#intro").click(button="right")
        expect(label).to_have_text("Comment:")
        textarea = overlay(page, "textarea")
        textarea.fill("On the paragraph.")
        textarea.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        page.locator("h1").click()
        page.keyboard.press("/")
        expect(label).to_have_text("Comment:")
        textarea.press("Escape")

        # Relative times move on.
        overlay(page, "#pill").click()
        where = overlay(page, "#outbox-list li .where")
        expect(where).to_have_text("#intro · just now")
        page.clock.fast_forward(61_000)
        expect(where).to_have_text("#intro · 1 min ago")
        overlay(page, "#collapse").click()

        # A comment on an answer.
        _ = jampot(tmp_path, "send", "An answer.")
        answer = overlay(page, ".agent-message")
        expect(answer).to_contain_text("An answer.")
        answer.click(button="right")
        expect(label).to_have_text("Comment on agent response:")


def show_cursor_on(page: Page) -> None:
    page.locator("h1").click()
    page.keyboard.press("x")
    expect(overlay(page, "#show-cursor")).to_have_attribute("aria-checked", "true")
    time.sleep(0.3)


def test_cursor_pages_follow_history_and_leaving(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SPA)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        overlay(alice_page, "#gear").click()
        expect(overlay(alice_page, "#here-count")).to_have_text("2 here")
        show_cursor_on(bob_page)

        def seen_again(x: int) -> None:
            bob_page.mouse.move(x, 250)
            bob_page.mouse.move(x + 5, 255)
            expect(cursors(alice_page)).to_have_count(1)

        seen_again(300)
        # replaceState moves bob away, and back.
        _ = bob_page.evaluate("history.replaceState({}, '', '/elsewhere')")
        expect(cursors(alice_page)).to_have_count(0)
        _ = bob_page.evaluate("history.replaceState({}, '', '/')")
        seen_again(310)
        # A hash change is another page; the back button (popstate)
        # returns.
        _ = bob_page.evaluate("location.hash = 'section'")
        expect(cursors(alice_page)).to_have_count(0)
        bob_page.go_back()
        seen_again(320)
        _ = bob_page.evaluate("history.pushState({}, '', '/pushed')")
        expect(cursors(alice_page)).to_have_count(0)
        bob_page.go_back()
        seen_again(330)

        # Leaving the page (pointerleave, a hidden tab) takes it away.
        _ = bob_page.evaluate(
            "document.documentElement.dispatchEvent(new PointerEvent('pointerleave'))"
        )
        expect(cursors(alice_page)).to_have_count(0)
        seen_again(340)
        _ = bob_page.evaluate(
            """() => {
                Object.defineProperty(document, "visibilityState", {
                    configurable: true, get: () => "hidden",
                });
                document.dispatchEvent(new Event("visibilitychange"));
                delete document.visibilityState;
            }"""
        )
        expect(cursors(alice_page)).to_have_count(0)
        seen_again(350)


def test_cursors_are_placed_on_the_element(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
    ):
        alice_context = browser.new_context(viewport={"width": 900, "height": 720})
        bob_context = browser.new_context(viewport={"width": 1280, "height": 720})
        try:
            alice_page = alice_context.new_page()
            _ = alice_page.goto(alice.url)
            bob_page = bob_context.new_page()
            _ = bob_page.goto(bob.url)
            overlay(alice_page, "#gear").click()
            expect(overlay(alice_page, "#here-count")).to_have_text("2 here")
            show_cursor_on(bob_page)

            def point(page: Page, x: float, y: float) -> tuple[float, float]:
                box = page.locator("#intro").bounding_box()
                assert box is not None
                return box["x"] + x * box["width"], box["y"] + y * box["height"]

            # The paragraph sits elsewhere in each page (centred, another
            # width): the cursor lands at the same spot of it.
            bob_x, bob_y = point(bob_page, 0.25, 0.5)
            bob_page.mouse.move(bob_x, bob_y)
            expect(cursors(alice_page)).to_have_count(1)
            alice_x, alice_y = point(alice_page, 0.25, 0.5)
            assert abs(alice_x - bob_x) > 50
            eventually("the same spot", lambda: near(alice_page, alice_x, alice_y))

            # A jump glides: positions in between are drawn.
            _ = alice_page.evaluate(
                """() => {
                        window.track = [];
                        const cursor = document.querySelector("#jampot")
                            .shadowRoot.querySelector(".cursor");
                        const sample = () => {
                            const x = parseFloat(
                                cursor.style.transform.replace("translate(", ""));
                            window.track.push(x);
                            if (window.track.length < 120) requestAnimationFrame(sample);
                        };
                        requestAnimationFrame(sample);
                    }"""
            )
            end_x, end_y = point(bob_page, 0.9, 0.5)
            bob_page.mouse.move(end_x, end_y)
            target_x, target_y = point(alice_page, 0.9, 0.5)
            eventually("the end", lambda: near(alice_page, target_x, target_y))
            alice_page.wait_for_timeout(300)
            seen = cast(list[float], alice_page.evaluate("window.track"))
            between = [x for x in seen if alice_x + 2 < x < target_x - 2]
            assert between, seen
        finally:
            alice_context.close()
            bob_context.close()


def post_settings(joined: Joined, body: str) -> None:
    status, answer = request(
        joined,
        "/_jampot/api/settings",
        "POST",
        {"Origin": f"http://{joined.host}", "Content-Type": "application/json"},
        body.encode(),
    )
    assert status == 200, answer


def hold_status_reads(page: Page, after: int = 0) -> list[tuple[Route, APIResponse]]:
    """Holds the page's api/status answers (after the first `after`)."""
    held: list[tuple[Route, APIResponse]] = []
    seen: list[int] = []

    def hold(route: Route) -> None:
        response = route.fetch()
        seen.append(1)
        if len(seen) <= after:
            route.fulfill(response=response)
        else:
            held.append((route, response))

    page.route("**/_jampot/api/status", hold)
    return held


def release(held: list[tuple[Route, APIResponse]]) -> None:
    for route, response in held:
        route.fulfill(response=response)
    held.clear()


def test_a_snapshot_is_not_lost_to_someone_elses_rename(
    tmp_path: Path, page: Page
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "bob", shared, "bob") as bob,
        join(tmp_path / "alice", shared, "alice") as alice,
    ):
        post_settings(bob, '{"commentMode":"off"}')
        # The version check passes; the page's first real read is held.
        held = hold_status_reads(page, after=1)
        _ = page.goto(bob.url)
        waiting(page, "the first status read", lambda: len(held) > 0)
        post_settings(alice, '{"name":"changed"}')
        expect(overlay(page, "#avatars")).to_contain_text("C")
        page.wait_for_timeout(300)
        release(held)
        page.unroute("**/_jampot/api/status")
        comment_mode = overlay(page, "#comment-mode")
        expect(comment_mode).to_have_attribute("aria-checked", "false")
        # The controls work: their settings were loaded.
        page.locator("h1").click()
        page.keyboard.press("c")
        expect(comment_mode).to_have_attribute("aria-checked", "true")


def test_a_recovery_snapshot_is_not_lost_to_someone_elses_rename(
    tmp_path: Path, page: Page
) -> None:
    port = free_port()
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
    ):
        with join(tmp_path / "bob", shared, "bob", port=port) as bob:
            _ = page.goto(bob.url)
            expect(overlay(page, "#connection")).to_have_attribute(
                "data-state", "green"
            )
            comment_on_intro(page, "Before the restart.")
            row = overlay(page, "#outbox-list li")
            expect(row).to_have_attribute("data-state", "queued")
        _ = jampot(tmp_path, "recv")
        held = hold_status_reads(page)
        with join(tmp_path / "bob", shared, "bob", port=port) as bob:
            expect(overlay(page, "#connection")).to_have_attribute(
                "data-state", "green", timeout=RECONNECT
            )
            waiting(page, "a recovery read", lambda: len(held) > 0)
            post_settings(alice, '{"name":"changed"}')
            expect(overlay(page, "#avatars")).to_contain_text("C")
            page.wait_for_timeout(300)
            release(held)
            page.unroute("**/_jampot/api/status")
            status = cast(
                dict[str, object], json.loads(request(bob, "/_jampot/api/status")[1])
            )
            assert status["pending"] == 0
            expect(row).to_have_attribute("data-state", "delivered")
            expect(overlay(page, "#badge")).to_have_text("0 queued")


def test_a_snapshot_converges_through_moving_cursors(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
    ):
        post_settings(bob, '{"commentMode":"off"}')
        with pages(browser, alice) as (alice_page,):
            alice_page.locator("h1").click()
            alice_page.keyboard.press("x")
            expect(overlay(alice_page, "#show-cursor")).to_have_attribute(
                "aria-checked", "true"
            )
            context = browser.new_context(viewport={"width": 1280, "height": 720})
            try:
                bob_page = context.new_page()
                held = hold_status_reads(bob_page, after=1)
                _ = bob_page.goto(bob.url)
                waiting(bob_page, "the first status read", lambda: len(held) > 0)
                # Alice's pointer keeps moving on the same page meanwhile.
                for step in range(40):
                    alice_page.mouse.move(200 + (step % 20) * 10, 250)
                    alice_page.wait_for_timeout(30)
                expect(cursors(bob_page)).to_have_count(1)
                release(held)
                for step in range(40):
                    alice_page.mouse.move(200 + (step % 20) * 10, 260)
                    alice_page.wait_for_timeout(30)
                expect(overlay(bob_page, "#comment-mode")).to_have_attribute(
                    "aria-checked", "false"
                )
            finally:
                context.close()


def test_a_stale_status_does_not_undo_an_event(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        dot = overlay(page, "#connection")
        expect(dot).to_have_attribute("data-state", "green")
        # What api/status said while the share was there.
        stale = request(joined, "/_jampot/api/status")[1]
        held: list[Route] = []
        page.route("**/_jampot/api/status", lambda route: held.append(route))
        # A delivery makes the page read api/status again; that answer is
        # held while the share goes away.
        comment_on_intro(page, "Any comment.")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        _ = jampot(tmp_path, "recv")
        waiting(page, "a status read", lambda: len(held) > 0)
        shared.process.stop()
        expect(dot).to_have_attribute("data-state", "grey")
        for route in held:
            route.fulfill(status=200, content_type="application/json", body=stale)
        page.wait_for_timeout(500)
        expect(dot).to_have_attribute("data-state", "grey")


def test_a_stale_status_does_not_undo_newer_state(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        page.wait_for_timeout(300)
        stale = request(joined, "/_jampot/api/status")[1]
        held: list[Route] = []
        page.route("**/_jampot/api/status", lambda route: held.append(route))
        # A delivery makes the page read api/status; that read is held.
        comment_on_intro(page, "First.")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        _ = jampot(tmp_path, "recv")
        waiting(page, "a status read", lambda: len(held) > 0)
        # Meanwhile: a new comment, and comment mode turned off.
        comment_on_intro(page, "Still pending.")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        page.locator("h1").click()
        page.keyboard.press("c")
        expect(overlay(page, "#comment-mode")).to_have_attribute(
            "aria-checked", "false"
        )
        # The old answer arrives last.
        for route in held:
            route.fulfill(status=200, content_type="application/json", body=stale)
        page.wait_for_timeout(500)
        page.unroute("**/_jampot/api/status")
        actual = cast(
            dict[str, object], json.loads(request(joined, "/_jampot/api/status")[1])
        )
        assert actual["pending"] == 1
        rows = overlay(page, "#outbox-list li")
        expect(rows.first).to_have_attribute("data-state", "queued")
        expect(rows.nth(1)).to_have_attribute("data-state", "delivered")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        expect(overlay(page, "#comment-mode")).to_have_attribute(
            "aria-checked", "false"
        )


def test_a_restarted_joiner_is_believed(tmp_path: Path, page: Page) -> None:
    port = free_port()
    with share(tmp_path, "serve", str(SITE)) as shared:
        with join(tmp_path / "bob", shared, "bob", port=port) as bob:
            _ = page.goto(bob.url)
            dot = overlay(page, "#connection")
            expect(dot).to_have_attribute("data-state", "green")
            # Move the joiner's view revision well past where a new joiner
            # starts.
            _ = page.evaluate(
                """async () => {
                    for (let index = 0; index < 60; index++) {
                        await fetch("/_jampot/api/settings", {
                            method: "POST",
                            headers: { "Content-Type": "application/json" },
                            body: JSON.stringify({ commentMode: "on" }),
                        });
                    }
                }"""
            )
            comment_on_intro(page, "Before the restart.")
            row = overlay(page, "#outbox-list li")
            expect(row).to_have_attribute("data-state", "queued")
        # Delivered while the joiner is away; a new joiner, same page.
        _ = jampot(tmp_path, "recv")
        with join(tmp_path / "bob", shared, "bob", port=port):
            expect(dot).to_have_attribute("data-state", "green", timeout=RECONNECT)
            expect(row).to_have_attribute("data-state", "delivered")
            # Its settings are believed too.
            page.locator("h1").click()
            page.keyboard.press("c")
            expect(overlay(page, "#comment-mode")).to_have_attribute(
                "aria-checked", "false"
            )


def test_a_late_settings_answer_does_not_undo_newer_settings(
    tmp_path: Path, page: Page
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        held: list[tuple[Route, APIResponse]] = []

        def hold(route: Route) -> None:
            held.append((route, route.fetch()))

        page.route("**/_jampot/api/settings", hold)
        page.locator("h1").click()
        page.keyboard.press("c")
        waiting(page, "the held answer", lambda: len(held) > 0)
        comment_mode = overlay(page, "#comment-mode")
        expect(comment_mode).to_have_attribute("aria-checked", "false")
        # Turned on again elsewhere (another page of the same joiner).
        _ = request(
            joined,
            "/_jampot/api/settings",
            "POST",
            {"Origin": f"http://{joined.host}", "Content-Type": "application/json"},
            json.dumps({"commentMode": "on"}).encode(),
        )
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        for route, response in held:
            route.fulfill(response=response)
        page.wait_for_timeout(300)
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        # And it behaves on: right-click comments.
        page.locator("#intro").click(button="right")
        expect(overlay(page, "textarea")).to_be_focused()


def test_a_delivery_before_the_queued_answer(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        held: list[tuple[Route, APIResponse]] = []

        def hold(route: Route) -> None:
            held.append((route, route.fetch()))

        page.route("**/_jampot/api/comments", hold)
        page.locator("#intro").click(button="right")
        overlay(page, "textarea").fill("Fast agent.")
        overlay(page, "textarea").press("Enter")
        waiting(page, "the queued answer", lambda: len(held) > 0)
        # The agent takes it before the page hears it was queued.
        _ = jampot(tmp_path, "recv")
        page.wait_for_timeout(300)
        for route, response in held:
            route.fulfill(response=response)
        overlay(page, "#pill").click()
        expect(overlay(page, "#outbox-list li")).to_have_attribute(
            "data-state", "delivered"
        )
        expect(overlay(page, "#badge")).to_have_text("0 queued")
