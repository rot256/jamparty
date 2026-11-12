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
import subprocess
import tempfile
import threading
import time
import tomllib
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

import pytest
from playwright.sync_api import Browser, Locator, Page, expect, sync_playwright

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
