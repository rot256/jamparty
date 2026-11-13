"""End-to-end tests: real `jampot serve`/`proxy` and `jampot join` processes.

Shares run with `--local` (no relays or address lookup) and joiners dial
them with `--share-addr`, so nothing leaves the machine.
"""

import base64
import hashlib
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import threading
import time
import tomllib
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

import pytest
from playwright.sync_api import (
    Browser,
    Locator,
    Page,
    WebSocketRoute,
    expect,
)
from support import (
    BINARY,
    ROOT,
    TIMEOUT,
    Joined,
    PageSocket,
    environment,
    hello,
    jampot,
    join,
    join_process,
    request,
    share,
)

SITE = ROOT / "tests/fixtures/site"
SPA = ROOT / "tests/fixtures/spa"
FIXTURE_HTML = (SITE / "index.html").read_bytes()
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


class EchoHandler(FixtureHandler):
    """A dev server that answers POST /echo with the headers it was sent."""

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        _ = self.rfile.read(length)
        names = ["host", "origin", "referer", "forwarded"]
        names += [
            name for name in self.headers if name.lower().startswith("x-forwarded")
        ]
        body = json.dumps({name.lower(): self.headers[name] for name in names}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        _ = self.wfile.write(body)


class StreamedHandler(BaseHTTPRequestHandler):
    """A dev server that streams its page in chunks, without a length."""

    protocol_version = "HTTP/1.1"

    def log_message(self, format: str, *_args: object) -> None:
        del format

    def do_GET(self) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        for chunk in [
            b"<!doctype html><html><body><h1>Streamed",
            b" page</h1><p id='intro'>In chunks.</p>",
            b"</body></html>",
        ]:
            _ = self.wfile.write(b"%x\r\n%s\r\n" % (len(chunk), chunk))
            self.wfile.flush()
            time.sleep(0.05)
        _ = self.wfile.write(b"0\r\n\r\n")


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
        assert status == 200 and b"api/socket" in body
        assert request(joined, "/missing")[0] == 404

        # Wrong Host (DNS rebinding) and cross-site reads are refused.
        assert (
            request(joined, "/", headers={"Host": f"evil.test:{joined.port}"})[0] == 421
        )
        cross = {"Sec-Fetch-Site": "cross-site", "Sec-Fetch-Mode": "cors"}
        assert request(joined, "/", headers=cross)[0] == 403
        # The page's socket opens only for the joiner's own origin.
        evil = PageSocket(joined, origin="http://evil.test")
        assert evil.status == 403
        evil.close()
        page = PageSocket(joined)
        try:
            assert page.status == 101
            assert page.next()["type"] == "hello"
            reply = page.request(
                {
                    "type": "comment",
                    "ref": "test-1",
                    "page": "/",
                    "selector": "h1",
                    "comment": "Hi",
                }
            )
            outcome = cast(dict[str, dict[str, str]], reply["outcome"])
            assert outcome["commented"]["id"]
        finally:
            page.close()
        assert hello(joined)["mine"] == [outcome["commented"]["id"]]
        assert request(joined, "/_jampot/api/status")[0] == 200
        # The rest of the API is reserved: nothing there reaches the site.
        for path in ["/_jampot/api/events", "/_jampot/api/missing"]:
            status, body = request(joined, path)
            assert status == 404, path
            assert b"no such jampot path" in body, body


BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def base58_decode(text: str) -> bytes:
    """A party token's bytes: the share's key (32) and the secret (16)."""
    value = 0
    for character in text:
        value = value * 58 + BASE58.index(character)
    zeros = len(text) - len(text.lstrip("1"))
    return bytes(zeros) + value.to_bytes((value.bit_length() + 7) // 8, "big")


def base58_encode(data: bytes) -> str:
    value = int.from_bytes(data, "big")
    digits = ""
    while value:
        value, digit = divmod(value, 58)
        digits = BASE58[digit] + digits
    zeros = len(data) - len(data.lstrip(b"\0"))
    return "1" * zeros + digits


def test_a_wrong_token_is_refused(tmp_path: Path) -> None:
    with share(tmp_path, "serve", str(SITE)) as shared:
        token = base58_decode(shared.token)
        assert len(token) == 48
        assert base58_encode(token) == shared.token
        # The share's own key with another secret.
        wrong = base58_encode(token[:32] + bytes(range(16)))
        with join_process(tmp_path, shared, wrong) as process:
            _ = process.wait_for("jampot: connecting to the share")
            # Refused before it serves anything: an error, no address.
            output = process.wait_for("jampot: the share refused")
            assert "token" in output
            assert process.process.wait(timeout=TIMEOUT) == 1
            assert not any(line.startswith("jampot: open") for line in process.seen)


# Dev servers on 127.0.0.2 stand for dev servers on another machine: the
# joiner's 127.0.0.1 and [::1] are free on their port, and pages reach
# them only through jampot.
DEV_HOST = "127.0.0.2"


def test_the_joiner_serves_on_the_dev_servers_port(tmp_path: Path, page: Page) -> None:
    with (
        upstream_server(DEV_HOST) as upstream,
        share(tmp_path, "proxy", upstream) as shared,
        join(tmp_path, shared) as joined,
    ):
        assert joined.port == urlsplit(upstream).port
        _ = page.goto(joined.url)
        expect(page.locator("h1")).to_have_text("Fixture page")
        expect(page.locator("#jampot")).to_be_attached()


def test_a_taken_port_falls_back_with_a_note(tmp_path: Path, page: Page) -> None:
    with upstream_server(DEV_HOST) as upstream:
        port = urlsplit(upstream).port
        assert port is not None
        with socket.socket(socket.AF_INET) as taken:
            # Something else here has the dev server's port.
            taken.bind(("127.0.0.1", port))
            taken.listen()
            with (
                share(tmp_path, "proxy", upstream) as shared,
                join_process(tmp_path, shared, shared.token) as process,
            ):
                note = process.wait_for("jampot: port ")
                assert note.startswith(f"{port} is in use here; serving on "), note
                assert "Live reload" in note
                url = process.wait_for("jampot: open ")
                served = urlsplit(url).port
                assert served is not None and served != port
                assert note.startswith(f"{port} is in use here; serving on {served}.")
                _ = page.goto(url)
                expect(page.locator("h1")).to_have_text("Fixture page")
                expect(page.locator("#jampot")).to_be_attached()


def test_the_dev_server_sees_its_own_origin(tmp_path: Path, page: Page) -> None:
    with (
        upstream_server(DEV_HOST, EchoHandler) as upstream,
        share(tmp_path, "proxy", upstream) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(f"{joined.url}?from=page")
        expect(page.locator("h1")).to_have_text("Fixture page")
        sent = cast(
            dict[str, str | None],
            page.evaluate(
                """async () => {
                    const response = await fetch("/echo", {
                        method: "POST",
                        headers: { "X-Forwarded-Host": "evil.example" },
                        body: "x",
                    });
                    return response.json();
                }"""
            ),
        )
        assert sent == {
            "host": urlsplit(upstream).netloc,
            "origin": upstream,
            "referer": f"{upstream}/?from=page",
            "forwarded": None,
        }


def test_streamed_html_gets_the_overlay(tmp_path: Path, page: Page) -> None:
    with (
        upstream_server(handler=StreamedHandler) as upstream,
        share(tmp_path, "proxy", upstream) as shared,
        join(tmp_path, shared) as joined,
    ):
        status, body = request(joined, "/")
        assert status == 200
        assert body.startswith(b"<!doctype html><html><body><h1>Streamed page</h1>")
        assert b"/_jampot/client.js?version=" in body
        _ = page.goto(joined.url)
        expect(page.locator("h1")).to_have_text("Streamed page")
        expect(page.locator("#jampot #connection")).to_have_attribute(
            "data-state", "green"
        )


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
    overview = jampot(tmp_path)
    for line in ["jampot serve <dir>", "jampot proxy <url>", "jampot join <token>"]:
        assert line in overview
    assert "jampot agent" in overview
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
    # Closed once the joiner answered.
    expect(page.locator("#jampot #jampot-dialog")).not_to_be_visible()


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


def recv_all(home: Path) -> list[dict[str, object]]:
    """Every comment the share holds, as `jampot recv` prints them."""
    return [
        cast(dict[str, object], json.loads(line))
        for line in jampot(home, "recv").splitlines()
    ]


def test_a_comment_draft_survives_reloads(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "site"
    _ = shutil.copytree(SITE, site)
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        page.locator("#intro").click(button="right")
        textarea = page.locator("#jampot textarea")
        expect(textarea).to_be_focused()
        textarea.press_sequentially("Half written")

        # A reload (the page's own, or a dev server's) brings it back.
        _ = page.reload()
        expect(textarea).to_have_value("Half written")
        expect(textarea).to_be_focused()
        expect(overlay(page, "#dialog-label")).to_have_text("Comment:")
        expect(overlay(page, "#status")).to_have_text("")
        textarea.press_sequentially(", now whole.")

        # jampot's own live reload waits for the dialog to close.
        index = site / "index.html"
        _ = index.write_text(index.read_text().replace("Fixture page", "Changed page"))
        expect(overlay(page, "#reload-notice")).to_be_visible()
        expect(textarea).to_have_value("Half written, now whole.")
        # Sent, then reloaded: the draft is gone with the dialog.
        with page.expect_navigation():
            textarea.press("Enter")
        expect(page.locator("h1")).to_have_text("Changed page")
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        expect(overlay(page, "#jampot-dialog")).not_to_be_visible()
        [record] = recv_all(tmp_path)
        assert record["comment"] == "Half written, now whole."
        assert record["selector"] == "#intro"


def test_a_draft_whose_element_is_gone_becomes_a_page_comment(
    tmp_path: Path, page: Page
) -> None:
    site = tmp_path / "site"
    _ = shutil.copytree(SITE, site)
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        page.locator("#intro").click(button="right")
        textarea = page.locator("#jampot textarea")
        textarea.press_sequentially("About that paragraph")
        index = site / "index.html"
        _ = index.write_text(index.read_text().replace('id="intro" ', ""))
        # The page has heard of the change: a reload gets the new one, and
        # no reload of jampot's own is left waiting for the dialog.
        expect(overlay(page, "#reload-notice")).to_be_visible()
        gone = "The element is gone; this will be a page comment."
        for _ in range(2):
            _ = page.reload()
            expect(textarea).to_have_value("About that paragraph")
            expect(overlay(page, "#dialog-label")).to_have_text("Comment:")
            expect(overlay(page, "#status")).to_have_text(gone)
            expect(overlay(page, "#reload-notice")).to_be_hidden()
        textarea.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "About that paragraph"
        assert record["selector"] == ""
        # Cancelled or sent, a draft is not brought back.
        _ = page.reload()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(overlay(page, "#jampot-dialog")).not_to_be_visible()


def test_a_draft_on_an_answer_comes_back_on_it(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jampot(tmp_path, "send", "An answer.")
        answer = overlay(page, ".agent-message")
        expect(answer).to_contain_text("An answer.")
        answer.click(button="right")
        textarea = overlay(page, "textarea")
        textarea.press_sequentially("About the answer")
        _ = page.reload()
        # Answers come with the first status: the draft waits for them.
        expect(textarea).to_have_value("About the answer")
        label = overlay(page, "#dialog-label")
        expect(label).to_have_text("Comment on agent response:")
        expect(overlay(page, "#status")).to_have_text("")
        textarea.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "About the answer"
        assert str(record["selector"]).startswith("jampot-agent:")

        # An answer dismissed meanwhile is gone: the reply goes to the page,
        # and says so, however often the page reloads.
        answer.click(button="right")
        textarea.press_sequentially("On a dismissed answer")
        message_id = answer.get_attribute("data-message-id")
        assert message_id is not None
        _ = page.evaluate(
            "(id) => sessionStorage.setItem('jampot-dismissed-v1', JSON.stringify([id]))",
            message_id,
        )
        for _ in range(2):
            _ = page.reload()
            expect(textarea).to_have_value("On a dismissed answer")
            expect(label).to_have_text("Comment on agent response:")
            expect(overlay(page, "#status")).to_have_text(
                "The element is gone; this will be a page comment."
            )
        textarea.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "On a dismissed answer"
        assert record["selector"] == ""


# Holds the comments the page sends its joiner until
# `window.jampotRelease()`; then sends them, or answers them with `outcome`
# (as the joiner's reply would) if given. `window.jampotRestore()` stops.
HOLD_COMMENTS = """(outcome) => {
    const original = WebSocket.prototype.send;
    let release;
    const released = new Promise((resolve) => { release = resolve; });
    window.jampotRelease = () => release();
    window.jampotRestore = () => { WebSocket.prototype.send = original; };
    WebSocket.prototype.send = function (data) {
        const message = JSON.parse(data);
        if (message.type !== "comment") return original.call(this, data);
        void released.then(() => {
            if (outcome === null) return original.call(this, data);
            const reply = { type: "reply", to: message.n, outcome };
            this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(reply) }));
        });
    };
}"""


def test_a_sending_dialog_stays_until_answered(tmp_path: Path, page: Page) -> None:
    """Escape and an outside click do not dismiss a dialog whose comment
    is on its way, so the next comment's draft is never one the sending
    one forgets."""
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(HOLD_COMMENTS, None)
        page.locator("#intro").click(button="right")
        textarea = overlay(page, "textarea")
        textarea.press_sequentially("First")
        textarea.press("Enter")
        dialog = overlay(page, "#jampot-dialog")
        status = overlay(page, "#status")
        expect(status).to_have_text("Queueing…")
        textarea.press("Escape")
        page.mouse.click(5, 5)
        # Enter again sends nothing more.
        textarea.press("Enter")
        expect(dialog).to_be_visible()
        expect(textarea).to_have_value("First")
        expect(status).to_have_text("Queueing…")

        _ = page.evaluate("() => window.jampotRelease()")
        expect(dialog).not_to_be_visible()
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        # The next comment is a draft again, and survives a reload.
        page.locator("#action").click(button="right")
        textarea.press_sequentially("Second")
        _ = page.reload()
        expect(textarea).to_have_value("Second")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "First"


def test_a_refused_comment_is_a_draft_again(tmp_path: Path, page: Page) -> None:
    """A comment the joiner refuses (too many) goes back to the dialog, and
    is a draft again: a reload keeps it."""
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(HOLD_COMMENTS, {"refused": {"error": "too many comments"}})
        page.locator("#intro").click(button="right")
        textarea = overlay(page, "textarea")
        textarea.press_sequentially("Refused")
        textarea.press("Enter")
        expect(overlay(page, "#status")).to_have_text("Queueing…")
        textarea.press("Escape")
        _ = page.evaluate("() => window.jampotRelease()")
        expect(overlay(page, "#status")).to_have_text("too many comments")
        expect(overlay(page, "#jampot-dialog")).to_be_visible()
        expect(overlay(page, "#outbox-list li")).to_have_count(0)
        _ = page.reload()
        expect(textarea).to_have_value("Refused")
        expect(overlay(page, "#badge")).to_have_text("0 queued")


def test_a_reload_during_sending_does_not_send_twice(
    tmp_path: Path, page: Page
) -> None:
    """The share took the comment, but the page reloaded before it heard:
    the comment is not a draft any more, and goes out once."""
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        page.locator("#intro").click(button="right")
        textarea = overlay(page, "textarea")
        textarea.press_sequentially("Exactly once")
        # The joiner's answer arrives, but the page never sees it.
        tap.hold.add("reply")
        textarea.press("Enter")
        waiting(page, "the joiner's answer", lambda: len(tap.held) > 0)
        tap.held.clear()
        tap.hold.clear()
        _ = page.reload()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(overlay(page, "#jampot-dialog")).not_to_be_visible()
        rows = overlay(page, "#outbox-list li")
        expect(rows).to_have_count(1)
        expect(rows).to_have_attribute("data-state", "queued")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "Exactly once"
        assert record["selector"] == "#intro"


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
        sockets: list[str] = []
        page.on("websocket", lambda socket: sockets.append(socket.url))
        _ = page.goto(joined.url)
        text = page.locator("#jampot #notice")
        expect(text).to_contain_text("your jampot join runs 9.9.9 (page API 999)")
        expect(text).to_contain_text(f"the share runs {VERSION}")
        expect(page.locator("#jampot #connection")).to_have_attribute(
            "data-state", "grey"
        )
        page.wait_for_timeout(500)
        assert sockets == [], sockets


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
        link = cast(dict[str, dict[str, str]], hello(alice)["link"])
        fingerprint = link["you"]["fingerprint"]
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


class SocketTap:
    """Routes a page's sockets to its joiner through the test, so messages
    from the joiner of the types in `hold` can be held back."""

    def __init__(self, page: Page) -> None:
        self.hold: set[str] = set()
        self.held: list[tuple[WebSocketRoute, str]] = []
        self.sockets: list[WebSocketRoute] = []
        page.route_web_socket("**/_jampot/api/socket", self._route)

    def _route(self, route: WebSocketRoute) -> None:
        self.sockets.append(route)
        server = route.connect_to_server()

        def from_joiner(message: str | bytes) -> None:
            kind = cast(dict[str, object], json.loads(message))["type"]
            if kind in self.hold:
                self.held.append((route, str(message)))
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
        _ = page.evaluate(HOLD_COMMENTS, {"unsent": {"error": "not reachable"}})
        _ = page.evaluate("() => window.jampotRelease()")
        comment_on_intro(page, "Third, kept for later.")
        overlay(page, "#pill").click()
        unsent = rows.nth(0)
        expect(unsent).to_have_attribute("data-state", "unsent")
        expect(unsent.locator(".state")).to_have_text("not sent")
        expect(unsent.locator(".state")).to_have_css("color", "rgb(255, 168, 168)")
        _ = page.evaluate("() => window.jampotRestore()")
        unsent.locator("button.again").click()
        expect(unsent).to_have_attribute("data-state", "queued")
        expect(overlay(page, "#list-count")).to_have_text("1 queued")


def switch(page: Page, name: str) -> Locator:
    return overlay(page, f"#{name}")


def test_edit_mode(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(
            """() => {
                window.pressed = 0;
                window.delegated = 0;
                document.querySelector("#action").addEventListener("click", () => {
                    window.pressed += 1;
                });
                document.addEventListener("click", (event) => {
                    if (event.target.closest("#action")) window.delegated += 1;
                }, true);
                const link = document.createElement("a");
                link.id = "away";
                link.href = "/elsewhere.html";
                link.textContent = "Elsewhere";
                document.querySelector("main").append(link);
            }"""
        )
        button = overlay(page, "#edit")
        outline = overlay(page, "#edit-highlight")
        dialog = overlay(page, "#jampot-dialog")
        expect(button).to_have_attribute("aria-pressed", "false")

        # On: the button turns blue; hovering outlines the element.
        button.click()
        expect(button).to_have_attribute("aria-pressed", "true")
        expect(button).to_have_css("background-color", "rgb(47, 128, 237)")
        intro = page.locator("#intro")
        intro.hover()
        expect(outline).to_be_visible()
        box = intro.bounding_box()
        drawn = outline.bounding_box()
        assert box is not None and drawn is not None
        assert abs(box["x"] - drawn["x"]) < 2 and abs(box["width"] - drawn["width"]) < 2

        # A left click comments instead of pressing the button, also for a
        # handler the page delegates from the document in the capture phase.
        page.locator("#action").click()
        expect(dialog).to_be_visible()
        expect(overlay(page, "#dialog-label")).to_have_text("Comment:")
        assert page.evaluate("() => window.pressed") == 0
        assert page.evaluate("() => window.delegated") == 0
        textarea = overlay(page, "textarea")
        textarea.fill("Make this button say what it does.")
        textarea.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        # Still in edit mode; a link does not navigate.
        expect(button).to_have_attribute("aria-pressed", "true")
        page.locator("#away").click()
        expect(dialog).to_be_visible()
        assert page.url == joined.url

        # Escape closes the dialog first, then leaves edit mode.
        page.keyboard.press("Escape")
        expect(dialog).not_to_be_visible()
        expect(button).to_have_attribute("aria-pressed", "true")
        page.keyboard.press("Escape")
        expect(button).to_have_attribute("aria-pressed", "false")
        expect(outline).to_be_hidden()

        # Off again: clicks are the page's.
        page.locator("#action").click()
        assert page.evaluate("() => window.pressed") == 1
        assert page.evaluate("() => window.delegated") == 1
        expect(dialog).not_to_be_visible()

        # The button also leaves it; the overlay works while it is on.
        button.click()
        overlay(page, "#pill").click()
        expect(overlay(page, "#list")).to_be_visible()
        expect(overlay(page, "#outbox-list li")).to_have_count(1)
        overlay(page, "#collapse").click()
        button.click()
        expect(button).to_have_attribute("aria-pressed", "false")
        page.locator("#action").click()
        assert page.evaluate("() => window.pressed") == 2


def test_edit_mode_follows_comment_mode(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(
            """() => {
                window.pressed = 0;
                document.querySelector("#action").addEventListener("click", () => {
                    window.pressed += 1;
                });
            }"""
        )
        button = overlay(page, "#edit")
        panel = overlay(page, "#panel")
        dialog = overlay(page, "#jampot-dialog")

        # In edit mode a press on the page still closes the open panel.
        overlay(page, "#gear").click()
        expect(panel).to_be_visible()
        button.click()
        expect(button).to_have_attribute("aria-pressed", "true")
        page.locator("#intro").click()
        expect(panel).to_be_hidden()
        page.keyboard.press("Escape")
        expect(dialog).not_to_be_visible()

        # Turning comment mode off leaves edit mode: clicks are the page's.
        page.keyboard.press("c")
        expect(switch(page, "comment-mode")).to_have_attribute("aria-checked", "false")
        expect(button).to_have_attribute("aria-pressed", "false")
        expect(button).to_be_disabled()
        page.locator("#action").click()
        assert page.evaluate("() => window.pressed") == 1
        expect(dialog).not_to_be_visible()
        # While it is off, edit mode cannot be entered.
        button.click(force=True)
        expect(button).to_have_attribute("aria-pressed", "false")
        page.locator("#action").click()
        assert page.evaluate("() => window.pressed") == 2

        # Comment mode on again: available again.
        page.keyboard.press("c")
        expect(button).to_be_enabled()
        button.click()
        expect(button).to_have_attribute("aria-pressed", "true")
        page.locator("#action").click()
        expect(dialog).to_be_visible()
        assert page.evaluate("() => window.pressed") == 2


def test_the_overlay_moves_to_any_corner(tmp_path: Path, page: Page) -> None:
    def place(selector: str) -> tuple[bool, bool]:
        """Whether `selector` sits in the left half and the top half."""
        size = page.viewport_size
        box = overlay(page, selector).bounding_box()
        assert size is not None and box is not None
        return (
            box["x"] + box["width"] / 2 < size["width"] / 2,
            box["y"] + box["height"] / 2 < size["height"] / 2,
        )

    def corner(name: str) -> Locator:
        return overlay(page, f'.corner[data-corner="{name}"]')

    with share(tmp_path, "serve", str(SITE)) as shared:
        with join(tmp_path, shared) as joined:
            _ = page.goto(joined.url)
            expect(overlay(page, "#connection")).to_have_attribute(
                "data-state", "green"
            )
            assert place("#bar") == (False, False)
            overlay(page, "#gear").click()
            expect(corner("bottom-right")).to_have_attribute("aria-pressed", "true")

            for name, left, top in [
                ("top-left", True, True),
                ("bottom-left", True, False),
                ("top-right", False, True),
            ]:
                if not overlay(page, "#panel").is_visible():
                    overlay(page, "#gear").click()
                corner(name).click()
                expect(corner(name)).to_have_attribute("aria-pressed", "true")
                expect(page.locator("#jampot")).to_have_attribute("data-corner", name)
                assert place("#bar") == (left, top)
                assert place("#panel") == (left, top)
                # The panel opens towards the middle: below a top bar, above
                # a bottom one.
                bar = overlay(page, "#bar").bounding_box()
                panel = overlay(page, "#panel").bounding_box()
                assert bar is not None and panel is not None
                if top:
                    assert panel["y"] >= bar["y"] + bar["height"]
                else:
                    assert panel["y"] + panel["height"] <= bar["y"]
                # The comment list goes to the same corner.
                overlay(page, "#gear").click()
                overlay(page, "#pill").click()
                expect(overlay(page, "#list")).to_be_visible()
                assert place("#list") == (left, top)
                overlay(page, "#collapse").click()

            # Top right now: the toast and unanchored answers go there too.
            _ = page.keyboard.press("/")
            textarea = overlay(page, "textarea")
            textarea.fill("A page comment.")
            textarea.press("Enter")
            expect(overlay(page, "#toast")).to_be_visible()
            assert place("#toast") == (False, True)
            _ = jampot(tmp_path, "send", "Thanks, looking at it.")
            answer = overlay(page, "#agent-general .agent-message")
            expect(answer).to_contain_text("Thanks, looking at it.")
            assert place("#agent-general .agent-message") == (False, True)

        # Saved on disk: a new joiner with the same home starts there.
        assert settings_file(tmp_path)["corner"] == "top-right"
        with join(tmp_path, shared) as joined:
            _ = page.goto(joined.url)
            expect(overlay(page, "#connection")).to_have_attribute(
                "data-state", "green"
            )
            expect(page.locator("#jampot")).to_have_attribute(
                "data-corner", "top-right"
            )
            assert place("#bar") == (False, True)


def test_outside_clicks_close_the_panel_and_the_list(
    tmp_path: Path, page: Page
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(
            """() => {
                window.clicks = 0;
                document.querySelector("h1").addEventListener("click", () => {
                    window.clicks += 1;
                });
            }"""
        )
        panel = overlay(page, "#panel")
        gear = overlay(page, "#gear")

        # The gear still toggles; a press inside the panel keeps it open.
        gear.click()
        expect(panel).to_be_visible()
        overlay(page, "#here-count").click()
        expect(panel).to_be_visible()
        gear.click()
        expect(panel).to_be_hidden()

        # A click on the page closes the panel, and the page still gets it.
        gear.click()
        expect(panel).to_be_visible()
        page.locator("h1").click()
        expect(panel).to_be_hidden()
        assert page.evaluate("() => window.clicks") == 1

        # The same for the comment list, which has no hint text when empty.
        overlay(page, "#pill").click()
        listed = overlay(page, "#list")
        expect(listed).to_be_visible()
        expect(listed).not_to_contain_text("No comments yet")
        expect(overlay(page, "#outbox-list li")).to_have_count(0)
        page.locator("h1").click()
        expect(listed).to_be_hidden()
        expect(overlay(page, "#bar")).to_be_visible()
        assert page.evaluate("() => window.clicks") == 2


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

        # Rebinding: the new key works, the old one does not. (The clicks on
        # the page closed the panel.)
        overlay(page, "#gear").click()
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


def waiting(page: Page, what: str, check: Callable[[], bool]) -> None:
    """Like `eventually`, letting `page` handle its events meanwhile."""
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        if check():
            return
        page.wait_for_timeout(50)
    pytest.fail(f"timed out waiting for {what}")


RECORD_SENT = """() => {
    window.jampotSent = [];
    const original = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
        window.jampotSent.push(JSON.parse(data));
        return original.call(this, data);
    };
}"""


def sent(page: Page, kind: str) -> list[dict[str, object]]:
    """The messages of type `kind` `page` sent its joiner since
    `RECORD_SENT`."""
    messages = cast(list[dict[str, object]], page.evaluate("window.jampotSent"))
    return [message for message in messages if message["type"] == kind]


def test_an_unrepresentable_url_is_nowhere(tmp_path: Path, browser: Browser) -> None:
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
        bob_page.mouse.move(300, 250)
        expect(cursors(alice_page)).to_have_count(1)

        # Too long to say: bob is nowhere, and his page sends nothing.
        _ = bob_page.evaluate("history.pushState({}, '', '/?' + 'q'.repeat(600))")
        expect(cursors(alice_page)).to_have_count(0)
        bob_page.wait_for_timeout(500)
        _ = bob_page.evaluate(RECORD_SENT)
        for step in range(10):
            bob_page.mouse.move(300 + step * 5, 260)
        bob_page.wait_for_timeout(500)
        assert sent(bob_page, "cursor") == []
        expect(cursors(alice_page)).to_have_count(0)

        # Back on a page it can name: there again.
        _ = bob_page.evaluate("history.pushState({}, '', '/')")
        time.sleep(0.3)
        bob_page.mouse.move(320, 270)
        expect(cursors(alice_page)).to_have_count(1)


def cursor_on(page: Page) -> None:
    page.locator("#title").click()
    page.keyboard.press("x")
    expect(overlay(page, "#show-cursor")).to_have_attribute("aria-checked", "true")
    page.wait_for_timeout(300)


def bob_is_at(bob: Joined, url: str) -> Callable[[], bool]:
    def check() -> bool:
        people = cast(list[dict[str, object]], hello(bob)["people"])
        return any(
            person["name"] == "bob" and person["url"] == url for person in people
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
                const url = new URL("/_jampot/api/socket", location.href);
                url.protocol = "ws:";
                const socket = new WebSocket(url);
                await new Promise((resolve) => { socket.onmessage = resolve; });
                for (let seq = 1; seq <= 65; seq++) {
                    socket.send(JSON.stringify({ type: "page", url: `/q${seq}` }));
                }
                window.burst = socket;
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


def test_movement_right_after_a_page_change_is_sent(
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
        # The page changes, and the pointer moves and stops at once: the
        # pointer follows the report on the socket, so it is on /other.
        _ = bob_page.evaluate("history.pushState({}, '', '/other')")
        bob_page.mouse.move(320, 260)
        expect(cursors(alice_page)).to_have_count(1)
        eventually("the cursor where bob stopped", lambda: near(alice_page, 320, 260))


def test_moving_pages_and_pointers_stay_connected(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SPA)) as shared,
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
        share(tmp_path, "serve", str(SPA)) as shared,
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

        # Off by default: bob's page sends nothing.
        _ = bob_page.evaluate(RECORD_SENT)
        for step in range(10):
            bob_page.mouse.move(200 + step * 10, 250)
        bob_page.wait_for_timeout(500)
        assert sent(bob_page, "cursor") == []

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
        bob_page.wait_for_timeout(300)
        _ = bob_page.evaluate("window.jampotSent = []")
        bob_page.wait_for_timeout(3000)
        assert sent(bob_page, "cursor") == []
        # Without updates it fades (on the receiver's own timer).
        expect(cursors(alice_page)).to_have_attribute("data-faded", "true")
        expect(cursors(alice_page)).to_have_css("opacity", "0")

        # A burst of movement: at most about 15 per second, ending on the
        # last position.
        _ = bob_page.evaluate("window.jampotSent = []")
        started = time.monotonic()
        for step in range(60):
            bob_page.mouse.move(200 + step * 5, 260)
            time.sleep(1 / 60)
        elapsed = time.monotonic() - started
        eventually("the final position", lambda: near(alice_page, 495, 260))
        bob_page.wait_for_timeout(300)
        count = len(sent(bob_page, "cursor"))
        assert 3 <= count <= 15 * elapsed + 2, (count, elapsed)

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

        # Bob turns it off: it disappears, and his page sends no more.
        bob_page.keyboard.press("x")
        expect(cursors(carol_page)).to_have_count(0)
        bob_page.wait_for_timeout(300)
        _ = bob_page.evaluate("window.jampotSent = []")
        for step in range(30):
            bob_page.mouse.move(250 + step * 5, 280)
        bob_page.wait_for_timeout(1000)
        assert sent(bob_page, "cursor") == []
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
        # What a socket of each page's joiner carries, as a page would see it.
        for page in (alice_page, bob_page):
            _ = page.evaluate(
                """() => {
                    window.seen = [];
                    const url = new URL("/_jampot/api/socket", location.href);
                    url.protocol = "ws:";
                    const socket = new WebSocket(url);
                    socket.onmessage = (event) => window.seen.push(event.data);
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

        # Messages arrive in order: once the marker is in, so is any delivery.
        def marked(page: Page) -> Callable[[], bool]:
            return lambda: any("Marker." in event for event in seen(page))

        for page in (alice_page, bob_page):
            waiting(page, "the marker", marked(page))
        mine = [
            cast(dict[str, object], json.loads(event))["undelivered"]
            for event in seen(alice_page)
            if '"type":"mine"' in event
        ]
        # Queued, then delivered.
        assert mine[-2:] == [[record["id"]], []], seen(alice_page)
        assert not any('"type":"mine"' in event for event in seen(bob_page)), seen(
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
            # The socket reconnects by itself and says where the page is now.
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


def test_a_restarted_joiner_is_believed(tmp_path: Path, page: Page) -> None:
    port = free_port()
    with share(tmp_path, "serve", str(SITE)) as shared:
        with join(tmp_path / "bob", shared, "bob", port=port) as bob:
            _ = page.goto(bob.url)
            dot = overlay(page, "#connection")
            expect(dot).to_have_attribute("data-state", "green")
            comment_on_intro(page, "Before the restart.")
            row = overlay(page, "#outbox-list li")
            expect(row).to_have_attribute("data-state", "queued")
        # Delivered while the joiner is away; the page cannot know yet.
        _ = jampot(tmp_path, "recv")
        page.wait_for_timeout(500)
        expect(row).to_have_attribute("data-state", "queued")
        # A new joiner, same page: it says what the share holds.
        with join(tmp_path / "bob", shared, "bob", port=port):
            expect(dot).to_have_attribute("data-state", "green", timeout=RECONNECT)
            expect(row).to_have_attribute("data-state", "delivered")
            # Its settings are believed too.
            page.locator("h1").click()
            page.keyboard.press("c")
            expect(overlay(page, "#comment-mode")).to_have_attribute(
                "aria-checked", "false"
            )


def test_a_restarted_joiner_keeps_what_is_still_pending(
    tmp_path: Path, page: Page
) -> None:
    port = free_port()
    with share(tmp_path, "serve", str(SITE)) as shared:
        with join(tmp_path / "bob", shared, "bob", port=port) as bob:
            _ = page.goto(bob.url)
            comment_on_intro(page, "Still waiting.")
            row = overlay(page, "#outbox-list li")
            expect(row).to_have_attribute("data-state", "queued")
        # Not delivered: the new joiner learns it from the share, and the
        # page keeps it queued.
        with join(tmp_path / "bob", shared, "bob", port=port):
            expect(overlay(page, "#connection")).to_have_attribute(
                "data-state", "green", timeout=RECONNECT
            )
            expect(overlay(page, "#badge")).to_have_text("1 queued")
            expect(row).to_have_attribute("data-state", "queued")
            [record] = recv_all(tmp_path)
            assert record["comment"] == "Still waiting."
            expect(row).to_have_attribute("data-state", "delivered")


def test_a_delivery_before_the_queued_answer(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        tap.hold.update({"reply", "mine"})
        page.locator("#intro").click(button="right")
        overlay(page, "textarea").fill("Fast agent.")
        overlay(page, "textarea").press("Enter")
        waiting(page, "the queued answer", lambda: len(tap.held) > 0)
        # The agent takes it before the page hears it was queued.
        _ = jampot(tmp_path, "recv")
        page.wait_for_timeout(300)
        tap.release()
        overlay(page, "#pill").click()
        expect(overlay(page, "#outbox-list li")).to_have_attribute(
            "data-state", "delivered"
        )
        expect(overlay(page, "#badge")).to_have_text("0 queued")


def test_a_second_tab_of_the_same_joiner(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        context = browser.new_context(viewport={"width": 1280, "height": 720})
        try:
            first, second = context.new_page(), context.new_page()
            for page in (first, second):
                _ = page.goto(joined.url)
                expect(overlay(page, "#connection")).to_have_attribute(
                    "data-state", "green"
                )
            # A comment from one tab counts in the other.
            comment_on_intro(first, "From the first tab.")
            expect(overlay(second, "#badge")).to_have_text("1 queued")
            # A setting changed in one is the other's too.
            overlay(second, "#gear").click()
            overlay(second, "#name-field").fill("robin")
            overlay(second, "#name-field").press("Enter")
            overlay(first, "#gear").click()
            expect(overlay(first, "#name-field")).to_have_value("robin")
            # Delivered: both tabs hear it.
            _ = jampot(tmp_path, "recv")
            for page in (first, second):
                expect(overlay(page, "#badge")).to_have_text("0 queued")
        finally:
            context.close()


def test_a_socket_the_joiner_closes_comes_back(tmp_path: Path, page: Page) -> None:
    """A page that fell behind (a frozen tab) is closed by its joiner; it
    connects again and starts from a fresh hello."""
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        dot = overlay(page, "#connection")
        expect(dot).to_have_attribute("data-state", "green")
        waiting(page, "the socket", lambda: len(tap.sockets) == 1)
        # Meanwhile the name changed elsewhere, and the joiner gives up.
        settings = PageSocket(joined)
        try:
            _ = settings.request({"type": "settings", "change": {"name": "robin"}})
        finally:
            settings.close()
        tap.hold.add("settings")
        tap.sockets[0].close(code=1013, reason="fell behind")
        tap.hold.clear()
        waiting(page, "a new socket", lambda: len(tap.sockets) == 2)
        expect(dot).to_have_attribute("data-state", "green", timeout=RECONNECT)
        overlay(page, "#gear").click()
        expect(overlay(page, "#name-field")).to_have_value("robin")
        comment_on_intro(page, "After the reconnect.")
        expect(overlay(page, "#badge")).to_have_text("1 queued")


def test_back_and_forward_reconnect(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        dot = overlay(page, "#connection")
        expect(dot).to_have_attribute("data-state", "green")
        _ = page.goto(f"{joined.url}?elsewhere")
        expect(dot).to_have_attribute("data-state", "green")
        _ = page.go_back()
        expect(dot).to_have_attribute("data-state", "green", timeout=RECONNECT)
        comment_on_intro(page, "After coming back.")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
