"""Presence and cursors across joiners and pages."""

import json
import re
import time
from collections.abc import Callable
from pathlib import Path
from typing import cast

import pytest
from playwright.sync_api import (
    Browser,
    Page,
    expect,
)
from support import (
    RECORD_SENT,
    SITE,
    SPA,
    Joined,
    cursor_on,
    cursors,
    eventually,
    hello,
    jamparty,
    join,
    overlay,
    pages,
    sent,
    share,
)


def cursor_at(page: Page) -> tuple[float, float] | None:
    """Where the one cursor drawn in `page` is, if any."""
    transform = cast(
        str | None,
        page.evaluate(
            """() => document.querySelector("#jamparty")
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


def test_an_unrepresentable_url_is_nowhere(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SPA)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        overlay(alice_page, "#gear").click()
        expect(overlay(alice_page, "#here-count")).to_have_text("2 here")
        cursor_on(bob_page)
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
                const url = new URL("/_jamparty/api/socket", location.href);
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
        cursor_on(bob_page)
        bob_page.mouse.move(300, 250)
        expect(cursors(alice_page)).to_have_count(1)
        expect(cursors(alice_page).locator(".tag")).to_have_text("bob")
        assert cursors(alice_page).locator(".tag").evaluate(
            "node => parseFloat(getComputedStyle(node).fontSize)"
        ) == pytest.approx(12.375, abs=0.02)
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
        [line] = jamparty(tmp_path, "recv").splitlines()
        assert cast(dict[str, object], json.loads(line))["selector"] == "#board"

        # Held still, nothing is sent.
        bob_page.wait_for_timeout(300)
        _ = bob_page.evaluate("window.jampartySent = []")
        bob_page.wait_for_timeout(3000)
        assert sent(bob_page, "cursor") == []
        # Without updates it fades (on the receiver's own timer).
        expect(cursors(alice_page)).to_have_attribute("data-faded", "true")
        expect(cursors(alice_page)).to_have_css("opacity", "0")

        # A burst of movement: at most about 15 per second, ending on the
        # last position.
        _ = bob_page.evaluate("window.jampartySent = []")
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
        _ = bob_page.evaluate("window.jampartySent = []")
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
                [...document.querySelector("#jamparty").shadowRoot
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
                "border-top-color", "rgb(32, 38, 50)"
            )
            bob_colour = avatars(alice_page)["bob"]
            assert bob_colour == avatars(carol_page)["bob"] != "rgb(255, 255, 255)"
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
        cursor_on(bob_page)

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
            cursor_on(bob_page)

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
                        const cursor = document.querySelector("#jamparty")
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
