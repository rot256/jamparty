"""Reloads and reconnections: live reload, joiners that stop and restart, sockets the joiner closes, more tabs, back and forward."""

import json
import shutil
import socket
import time
from pathlib import Path
from typing import cast

from playwright.sync_api import (
    Browser,
    Page,
    expect,
)
from support import (
    SITE,
    SPA,
    PageSocket,
    SocketTap,
    comment_on_intro,
    cursor_on,
    cursors,
    jampot,
    join,
    overlay,
    pages,
    recv_all,
    share,
    waiting,
)


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
                cursor_on(bob_page)
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
