"""Sharing a site: serving and proxying through a joiner, its ports, live reload, and the bare command."""

import http.client
import json
import socket
import time
from errno import EADDRINUSE
from http.server import BaseHTTPRequestHandler
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

from playwright.sync_api import (
    Page,
    expect,
)
from support import (
    ROOT,
    SITE,
    TIMEOUT,
    FixtureHandler,
    PageSocket,
    eventually,
    hello,
    jamparty,
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
        assert b"/_jamparty/client.js?live=0" in body
        status, body = request(joined, "/_jamparty/client.js")
        assert status == 200 and body == (ROOT / "assets/client.js").read_bytes()
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
        # The rest of the API is reserved: nothing there reaches the site.
        for path in [
            "/_jamparty/api/status",
            "/_jamparty/api/events",
            "/_jamparty/api/missing",
        ]:
            status, body = request(joined, path)
            assert status == 404, path
            assert b"no such jamparty path" in body, body


# Dev servers on 127.0.0.2 stand for dev servers on another machine: the
# joiner's 127.0.0.1 and [::1] are free on their port, and pages reach
# them only through jamparty.
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
        expect(page.locator("#jamparty")).to_be_attached()


def test_a_taken_port_falls_back_with_a_note(tmp_path: Path, page: Page) -> None:
    for attempt in range(5):
        with upstream_server(DEV_HOST) as upstream:
            port = urlsplit(upstream).port
            assert port is not None
            with socket.socket(socket.AF_INET) as taken:
                # Something else here has the dev server's port.
                try:
                    taken.bind(("127.0.0.1", port))
                except OSError as error:
                    if error.errno != EADDRINUSE or attempt == 4:
                        raise
                    continue
                taken.listen()
                with (
                    share(tmp_path, "proxy", upstream) as shared,
                    join_process(tmp_path, shared, shared.token) as process,
                ):
                    note = process.wait_for("jamparty: port ")
                    assert note.startswith(f"{port} is in use here; serving on "), note
                    assert "Live reload" in note
                    url = process.wait_for("jamparty: open ")
                    served = urlsplit(url).port
                    assert served is not None and served != port
                    assert note.startswith(
                        f"{port} is in use here; serving on {served}."
                    )
                    _ = page.goto(url)
                    expect(page.locator("h1")).to_have_text("Fixture page")
                    expect(page.locator("#jamparty")).to_be_attached()
                return


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
        assert b'/_jamparty/client.js"' in body
        _ = page.goto(joined.url)
        expect(page.locator("h1")).to_have_text("Streamed page")
        expect(page.locator("#jamparty #connection")).to_have_attribute(
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
        assert b"/_jamparty/client.js" in body
        _ = page.goto(joined.url)
        expect(page.locator("link[rel~=icon]")).to_have_count(0)
        echoed = page.evaluate(
            """() => new Promise((resolve, reject) => {
                const socket = new WebSocket(`ws://${location.host}/socket`);
                socket.onopen = () => socket.send("hello over iroh");
                socket.onmessage = (event) => resolve(event.data);
                socket.onerror = () => reject(new Error("websocket failed"));
            })"""
        )
        assert echoed == "hello over iroh"


def test_bare_jamparty_and_the_agent_guides(tmp_path: Path) -> None:
    overview = jamparty(tmp_path)
    for line in [
        "jamparty serve <path>",
        "jamparty proxy <url>",
        "jamparty join <token>",
    ]:
        assert line in overview
    assert "jamparty agent" in overview
    guides = jamparty(tmp_path, "agent")
    for kind in ["t3", "claude", "codex", "other"]:
        assert f"jamparty agent {kind}" in guides
    assert "jamparty recv" in jamparty(tmp_path, "agent", "claude")
    t3 = jamparty(tmp_path, "agent", "t3")
    assert 't3 thread send "$T3CODE_THREAD_ID" -' in t3
    assert "run `jamparty recv`" in t3
    help_text = jamparty(tmp_path, "--help")
    assert "serve" in help_text and "recv" not in help_text


def test_the_joiner_answers_the_client_itself(tmp_path: Path) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        # With the share gone the site is not reachable, but its client is.
        shared.process.stop()

        def disconnected() -> bool:
            return cast(dict[str, object], hello(joined)["link"])["connected"] is False

        eventually("the joiner to lose the share", disconnected)
        connection = http.client.HTTPConnection(
            "127.0.0.1", joined.port, timeout=TIMEOUT
        )
        try:
            connection.request(
                "GET", "/_jamparty/client.js?live=3", headers={"Host": joined.host}
            )
            response = connection.getresponse()
            body = response.read()
        finally:
            connection.close()
        assert response.status == 200
        assert response.getheader("Cache-Control") == "no-store"
        assert response.getheader("Content-Type") == "text/javascript; charset=utf-8"
        assert body == (ROOT / "assets/client.js").read_bytes()


def test_directory_listing_navigation_and_reload(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "site"
    sub = site / "sub #"
    sub.mkdir(parents=True)
    name = 'notes "draft"#?%.html'
    _ = (sub / name).write_text("<h1>Review notes</h1>")
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(
            page.get_by_role("heading", name="Index of /", exact=True)
        ).to_be_visible()
        page.get_by_role("link", name="sub #/").click()
        expect(page.get_by_role("heading", name="Index of /sub #")).to_be_visible()
        expect(page.locator("#jamparty #connection")).to_have_attribute(
            "data-state", "green"
        )
        _ = (sub / "new.txt").write_text("New file")
        expect(page.get_by_role("link", name="new.txt")).to_be_visible()
        page.get_by_role("link", name=name).click()
        expect(page.get_by_role("heading", name="Review notes")).to_be_visible()
