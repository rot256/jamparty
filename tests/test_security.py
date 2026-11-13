"""Who may use a joiner: the party secret, the page socket from other sites and under a strict CSP, and another page API."""

import json
import tomllib
from pathlib import Path
from typing import cast

from playwright.sync_api import (
    Page,
    expect,
)
from support import (
    ROOT,
    SITE,
    TIMEOUT,
    FixtureHandler,
    comment_on_intro,
    join,
    join_process,
    recv_all,
    share,
    upstream_server,
)

VERSION = cast(
    str, tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
)


class StrictHandler(FixtureHandler):
    """A dev server whose page allows connections to its own origin only."""

    def end_headers(self) -> None:
        self.send_header(
            "Content-Security-Policy",
            "default-src 'self'; connect-src 'self'; style-src 'self' 'unsafe-inline'",
        )
        super().end_headers()


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


def test_a_site_that_allows_only_its_own_origin(tmp_path: Path, page: Page) -> None:
    """`connect-src 'self'` covers the joiner's socket on the same host
    (and port), so the overlay works on such a site."""
    with (
        upstream_server(handler=StrictHandler) as upstream,
        share(tmp_path, "proxy", upstream) as shared,
        join(tmp_path, shared) as joined,
    ):
        violations: list[str] = []
        page.on(
            "console",
            lambda message: (
                violations.append(message.text)
                if "Content Security Policy" in message.text
                else None
            ),
        )
        _ = page.goto(joined.url)
        expect(page.locator("#jampot #connection")).to_have_attribute(
            "data-state", "green"
        )
        comment_on_intro(page, "Under a strict policy.")
        expect(page.locator("#jampot #badge")).to_have_text("1 queued")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "Under a strict policy."
        assert violations == [], violations


def test_another_site_cannot_open_the_socket(tmp_path: Path, page: Page) -> None:
    with (
        upstream_server() as upstream,
        share(tmp_path, "proxy", upstream) as shared,
        join(tmp_path, shared) as joined,
    ):
        # A page on another origin (the dev server's own) tries.
        _ = page.goto(upstream)
        opened = page.evaluate(
            """(url) => new Promise((resolve) => {
                const socket = new WebSocket(url);
                socket.onopen = () => resolve("opened");
                socket.onerror = () => resolve("refused");
            })""",
            f"ws://{joined.host}/_jampot/api/socket",
        )
        assert opened == "refused"


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
