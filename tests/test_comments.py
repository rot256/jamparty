"""Comments: the dialog, drafts across reloads, sending once, the comment list, delivery."""

import json
import shutil
from collections.abc import Callable
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

from playwright.sync_api import (
    Browser,
    Page,
    expect,
)
from support import (
    SITE,
    SocketTap,
    comment_on_intro,
    jampot,
    join,
    overlay,
    pages,
    recv_all,
    share,
    waiting,
)


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


def with_status(message: dict[str, object]) -> dict[str, object]:
    """`message` from the joiner as if the share had a status line."""
    line = "delivery failing, retrying"
    if message["type"] == "hello":
        link = cast(dict[str, object], message["link"])
        return {**message, "link": {**link, "status": line}}
    if message["type"] == "link":
        return {**message, "status": line}
    return message


def test_unsent_comments_go_out_under_a_status_line(tmp_path: Path, page: Page) -> None:
    """A comment the page could not send goes out again once the joiner is
    back, even while the share's status line shows."""
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        # The answer never reaches the page, and its socket goes: unsent.
        tap.hold.add("reply")
        page.locator("#intro").click(button="right")
        textarea = overlay(page, "textarea")
        textarea.press_sequentially("Despite the status line.")
        textarea.press("Enter")
        waiting(page, "the joiner's answer", lambda: len(tap.held) > 0)
        tap.held.clear()
        tap.hold.clear()
        tap.edit = with_status
        tap.sockets[-1].close()
        rows = overlay(page, "#outbox-list li")
        expect(rows).to_have_attribute("data-state", "queued")
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "grey")
        expect(overlay(page, "#connection-text")).to_contain_text(
            "delivery failing, retrying"
        )
        [record] = recv_all(tmp_path)
        assert record["comment"] == "Despite the status line."


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
