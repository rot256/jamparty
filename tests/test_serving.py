"""Sharing a site: serving and proxying through a joiner, its ports, live reload, and the bare command."""

import json
import socket
import time
from http.server import BaseHTTPRequestHandler
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

from playwright.sync_api import (
    Page,
    expect,
)
from support import (
    SITE,
    FixtureHandler,
    PageSocket,
    hello,
    jampot,
    join,
    join_process,
    request,
    share,
    upstream_server,
)


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
