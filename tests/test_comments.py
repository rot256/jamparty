"""Comments: the dialog, drafts across reloads, sending once, the comment list, delivery."""

import json
import shutil
from collections.abc import Callable
from hashlib import sha3_256
from pathlib import Path
from typing import cast
from urllib.parse import urlsplit

import pytest
from playwright.sync_api import (
    Browser,
    Page,
    Route,
    expect,
)
from support import (
    SITE,
    SocketTap,
    comment_on_intro,
    hello,
    jamparty,
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
        badge = page.locator("#jamparty #badge")
        expect(badge).to_have_text("0 queued")
        page.locator("#intro").click(button="right")
        textarea = page.locator("#jamparty textarea")
        expect(textarea).to_be_focused()
        textarea.fill("Tighten this paragraph.")
        textarea.press("Enter")
        expect(badge).to_have_text("1 queued")
        expect(page.locator("#jamparty #toast")).to_contain_text("Comment queued")


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
            entry = alice_page.locator("#jamparty #outbox-list li")
            expect(entry).to_have_attribute("data-state", "queued")

            output = jamparty(tmp_path, "recv")
            [line] = output.splitlines()
            record = cast(dict[str, object], json.loads(line))
            assert record["comment"] == "Shorter, please."
            assert record["selector"] == "#intro"
            expect(entry).to_have_attribute("data-state", "delivered")
            expect(alice_page.locator("#jamparty #badge")).to_have_text("0 queued")

            _ = jamparty(
                tmp_path,
                "send",
                "Done: **shorter** now.",
                "--anchor",
                str(record["id"]),
                "--to",
                "all",
            )
            for page in [alice_page, bob_page]:
                answer = page.locator("#jamparty .agent-message")
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
        page.clock.install()
        _ = page.goto(joined.url)
        indicator = page.locator("#jamparty #connection")
        expect(indicator).to_have_attribute("data-state", "green")
        shared.process.stop()
        expect(indicator).to_have_attribute("data-state", "grey")
        expect(page.locator("#jamparty #connection-text")).to_contain_text(
            "unreachable"
        )

        comment_on_intro(page, "Kept for later.")
        entry = page.locator("#jamparty #outbox-list li")
        expect(entry).to_have_attribute("data-state", "unsent")
        expect(entry.locator("button")).to_have_text("send again")

        # The list's periodic refresh keeps keyboard focus on the button.
        page.locator("#jamparty #pill").click()
        again = entry.locator("button.again")
        again.focus()
        expect(again).to_be_focused()
        page.clock.run_for(31_000)
        expect(again).to_be_focused()
        # Sending again replaces the button; focus falls back to the row.
        again.press("Enter")
        expect(entry).to_be_focused()


FILL_OUTBOX = """([count, key]) => {
    const entries = Array.from({ length: count }, (_, index) => ({
        draft: { page: location.href, selector: "#intro", selectedText: null, comment: `old ${index}` },
        ref: `old-${index}`,
        id: null,
        state: "unsent",
        key,
        at: Date.now(),
    }));
    const existing = JSON.parse(sessionStorage.getItem("jamparty-outbox-v1") ?? "[]");
    sessionStorage.setItem("jamparty-outbox-v1", JSON.stringify([...existing, ...entries]));
}"""


@pytest.mark.parametrize("with_refusal", [False, True])
def test_a_full_outbox_refuses_a_new_comment(
    tmp_path: Path, page: Page, with_refusal: bool
) -> None:
    """Comments still to send are never dropped: with the outbox full of
    them, a new one is refused, visibly, and stays a draft."""
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        # Sent again on load, but never answered: all still to send.
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        link = cast(dict[str, object], hello(joined)["link"])
        key = cast(dict[str, object], link["you"])["key"]
        if with_refusal:
            page.route(
                UPLOAD,
                lambda route: route.fulfill(status=507, json={"error": "storage full"}),
            )
            send_with_files(page, attachment_files(tmp_path, 1))
            expect(overlay(page, "#badge")).to_have_text("0 queued · 1 refused")
        _ = page.evaluate(FILL_OUTBOX, [19 if with_refusal else 20, key])
        tap.hold = {"comment_reply"}
        _ = page.reload()
        rows = overlay(page, "#outbox-list li")
        expect(rows).to_have_count(20)
        page.locator("#intro").click(button="right")
        textarea = overlay(page, "textarea")
        textarea.press_sequentially("One too many")
        textarea.press("Enter")
        expect(overlay(page, "#status")).to_contain_text(
            "The comment list is full" if with_refusal else "waiting to be sent"
        )
        expect(overlay(page, "#jamparty-dialog")).to_be_visible()
        expect(textarea).to_have_value("One too many")
        expect(rows).to_have_count(20)
        expect(rows.filter(has_text="old 0")).to_have_count(1)
        if with_refusal:
            refused = rows.filter(has_text="storage full")
            expect(refused).to_have_attribute("data-state", "failed")
            assert page.evaluate(FILE_STORE, "count") == 1
            textarea.press("Escape")
            overlay(page, "#pill").click()
            refused.get_by_role("button", name="edit").click()
            expect(overlay(page, "#attachments .attachment-name")).to_have_text(
                "attachment-0.txt"
            )


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
        textarea = page.locator("#jamparty textarea")
        expect(textarea).to_be_focused()
        textarea.press_sequentially("Half written")

        # A reload (the page's own, or a dev server's) brings it back.
        _ = page.reload()
        expect(textarea).to_have_value("Half written")
        expect(textarea).to_be_focused()
        expect(overlay(page, "#dialog-label")).to_have_text("Comment:")
        expect(overlay(page, "#status")).to_have_text("")
        textarea.press_sequentially(", now whole.")

        # jamparty's own live reload waits for the dialog to close.
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
        expect(overlay(page, "#jamparty-dialog")).not_to_be_visible()
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
        textarea = page.locator("#jamparty textarea")
        textarea.press_sequentially("About that paragraph")
        index = site / "index.html"
        _ = index.write_text(index.read_text().replace('id="intro" ', ""))
        # The page has heard of the change: a reload gets the new one, and
        # no reload of jamparty's own is left waiting for the dialog.
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
        expect(overlay(page, "#jamparty-dialog")).not_to_be_visible()


# Holds the comments the page sends its joiner until
# `window.jampartyRelease()`; then sends them, or answers them with `outcome`
# (as the joiner's reply would) if given. `window.jampartyRestore()` stops.
HOLD_COMMENTS = """(outcome) => {
    const original = WebSocket.prototype.send;
    let release;
    const released = new Promise((resolve) => { release = resolve; });
    window.jampartyRelease = () => release();
    window.jampartyRestore = () => { WebSocket.prototype.send = original; };
    WebSocket.prototype.send = function (data) {
        const message = JSON.parse(data);
        if (message.type !== "comment") return original.call(this, data);
        void released.then(() => {
            if (outcome === null) return original.call(this, data);
            const reply = { type: "comment_reply", to: message.n, outcome };
            this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(reply) }));
        });
    };
}"""


def test_comment_sends_in_background(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(HOLD_COMMENTS, None)
        comment_on_intro(page, "First")
        expect(overlay(page, "#badge")).to_have_text("0 queued · 1 sending")
        overlay(page, "#pill").click()
        row = overlay(page, "#outbox-list li")
        expect(row).to_have_attribute("data-state", "sending")
        spinner = row.locator(".sending-indicator")
        expect(spinner).to_be_visible()
        expect(spinner).to_have_css("animation-name", "jamparty-sending")
        page.emulate_media(reduced_motion="reduce")
        expect(spinner).to_have_css("animation-name", "none")

        overlay(page, "#collapse").click()
        page.locator("#action").click(button="right")
        textarea = overlay(page, "textarea")
        textarea.fill("Second")
        _ = page.evaluate("() => window.jampartyRelease()")
        expect(row).to_have_attribute("data-state", "queued")
        expect(textarea).to_have_value("Second")
        _ = page.reload()
        expect(textarea).to_have_value("Second")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "First"
        expect(row).to_have_attribute("data-state", "delivered")


@pytest.mark.parametrize("hash_route", [False, True])
def test_refused_comment_can_be_edited_or_discarded(
    tmp_path: Path, page: Page, hash_route: bool
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        original = f"{joined.url}#one" if hash_route else joined.url
        _ = page.goto(original)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(HOLD_COMMENTS, {"refused": {"error": "too many comments"}})
        comment_on_intro(page, "Refused")
        _ = page.evaluate("() => window.jampartyRelease()")
        expect(overlay(page, "#badge")).to_have_text("0 queued · 1 refused")
        overlay(page, "#pill").click()
        row = overlay(page, "#outbox-list li")
        expect(row).to_have_attribute("data-state", "failed")
        expect(row).to_contain_text("too many comments")
        other = (
            f"{joined.url}#two"
            if hash_route
            else f"{joined.url.rstrip('/')}/index.html"
        )
        _ = page.goto(other)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        if hash_route:
            _ = page.evaluate("""() => {
                const intro = document.querySelector("#intro");
                intro.id = "";
                // The route mounts its target after a fetch, not at once.
                window.addEventListener("hashchange", () => {
                    if (location.hash === "#one") {
                        setTimeout(() => { intro.id = "intro"; }, 400);
                    }
                });
            }""")
        else:
            overlay(page, "#pill").click()
        row.get_by_role("button", name="edit").click()
        expect(page).to_have_url(original)
        textarea = overlay(page, "textarea")
        expect(textarea).to_have_value("Refused")
        expect(overlay(page, "#highlight")).to_be_visible()
        textarea.fill("Edited after navigation")
        _ = page.reload()
        expect(textarea).to_have_value("Edited after navigation")
        expect(overlay(page, "#highlight")).to_be_visible()
        textarea.press("Escape")
        overlay(page, "#pill").click()
        row.get_by_role("button", name="discard").click()
        expect(row).to_have_count(0)
        expect(overlay(page, "#list-count")).to_have_text("0 queued")


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
        tap.hold.add("comment_reply")
        textarea.press("Enter")
        waiting(page, "the joiner's answer", lambda: len(tap.held) > 0)
        expect(overlay(page, "#jamparty-dialog")).to_be_hidden()
        expect(overlay(page, "#outbox-list li")).to_have_attribute(
            "data-state", "sending"
        )
        tap.held.clear()
        tap.hold.clear()
        _ = page.reload()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(overlay(page, "#jamparty-dialog")).not_to_be_visible()
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
        tap.hold.add("comment_reply")
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
        expect(overlay(page, "#connection-text")).to_have_text("· connected · direct")
        rows = overlay(page, "#outbox-list li")
        expect(rows).to_have_count(2)
        expect(rows.nth(0).locator(".where")).to_have_text("page · just now")
        expect(rows.nth(0).locator(".text")).to_have_text("Second, on the page.")
        expect(rows.nth(1).locator(".where")).to_have_text("#intro · just now")
        expect(rows.nth(1).locator(".state")).to_have_text("queued")

        # Delivered once the agent has them, in green.
        assert len(jamparty(tmp_path, "recv").splitlines()) == 2
        expect(rows.nth(1).locator(".state")).to_have_text("delivered")
        expect(rows.nth(1).locator(".state")).to_have_css("color", "rgb(23, 102, 48)")
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
        _ = page.evaluate("() => window.jampartyRelease()")
        comment_on_intro(page, "Third, kept for later.")
        overlay(page, "#pill").click()
        unsent = rows.nth(0)
        expect(unsent).to_have_attribute("data-state", "unsent")
        expect(unsent.locator(".state")).to_have_text("not sent")
        expect(unsent.locator(".state")).to_have_css("color", "rgb(162, 34, 34)")
        _ = page.evaluate("() => window.jampartyRestore()")
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
                    const url = new URL("/_jamparty/api/socket", location.href);
                    url.protocol = "ws:";
                    const socket = new WebSocket(url);
                    socket.onmessage = (event) => window.seen.push(event.data);
                }"""
            )
        comment_on_intro(alice_page, "Alice's comment.")
        expect(overlay(alice_page, "#badge")).to_have_text("1 queued")
        [line] = jamparty(tmp_path, "recv").splitlines()
        record = cast(dict[str, object], json.loads(line))
        assert record["comment"] == "Alice's comment."
        _ = jamparty(tmp_path, "send", "Marker.", "--to", "all")

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
        # The clock jump also trips the socket's idle watchdog.
        with page.expect_websocket():
            page.clock.fast_forward(61_000)
        expect(where).to_have_text("#intro · 1 min ago")
        overlay(page, "#collapse").click()

        # A comment on an answer received after reconnect.
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(tmp_path, "send", "An answer.")
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
        tap.hold.update({"comment_reply", "mine"})
        page.locator("#intro").click(button="right")
        overlay(page, "textarea").fill("Fast agent.")
        overlay(page, "textarea").press("Enter")
        waiting(page, "the queued answer", lambda: len(tap.held) > 0)
        # The agent takes it before the page hears it was queued.
        _ = jamparty(tmp_path, "recv")
        page.wait_for_timeout(300)
        tap.release()
        overlay(page, "#pill").click()
        expect(overlay(page, "#outbox-list li")).to_have_attribute(
            "data-state", "delivered"
        )
        expect(overlay(page, "#badge")).to_have_text("0 queued")


def test_a_comment_before_a_new_tabs_first_hello_is_sent(
    tmp_path: Path, page: Page
) -> None:
    """A comment written before a fresh tab's first hello is made with no
    joiner key known; it never reached a joiner, so it goes out under the
    key the hello brings."""
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        tap.hold.add("hello")
        _ = page.goto(joined.url)
        waiting(page, "the held hello", lambda: len(tap.held) > 0)
        comment_on_intro(page, "Before the first hello")
        rows = overlay(page, "#outbox-list li")
        expect(rows).to_have_attribute("data-state", "unsent")
        tap.release()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(rows).to_have_attribute("data-state", "queued")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "Before the first hello"


def test_a_nudge_never_delivers_comments(tmp_path: Path, page: Page) -> None:
    notice = tmp_path / "notice"
    with (
        share(
            tmp_path, "serve", str(SITE), "--", "sh", "-c", 'cat >> "$0"', str(notice)
        ) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        comment_on_intro(page, "Only recv may read this comment.")
        waiting(
            page,
            "the nudge",
            lambda: notice.exists() and "run jamparty recv" in notice.read_text(),
        )
        text = notice.read_text()
        assert text.startswith("jamparty: 1 comment waiting on ")
        assert "; run jamparty recv --share " in text
        assert "Only recv" not in text
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        expect(overlay(page, "#outbox-list li")).to_have_attribute(
            "data-state", "queued"
        )
        [record] = recv_all(tmp_path)
        assert record["comment"] == "Only recv may read this comment."
        expect(overlay(page, "#badge")).to_have_text("0 queued")
        expect(overlay(page, "#outbox-list li")).to_have_attribute(
            "data-state", "delivered"
        )


def test_file_attachments_survive_a_reload_after_sending(
    tmp_path: Path, page: Page
) -> None:
    data = b"\x00\xfetext\n"
    path = tmp_path / "report.bin"
    _ = path.write_bytes(data)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        page.locator("#intro").click(button="right")
        with page.expect_file_chooser() as chooser:
            overlay(page, "#attach").click()
        chooser.value.set_files(path)
        expect(overlay(page, "#attachments .attachment-name")).to_have_text(
            "report.bin"
        )
        expect(overlay(page, "#attachments .attachment-size")).to_have_text("7 B")

        # Paste and drop use the same chips. Remove those files before sending.
        _ = overlay(page, "#jamparty-dialog").evaluate(
            """dialog => {
                const paste = new DataTransfer();
                paste.items.add(new File(["paste"], "paste.txt"));
                const event = new ClipboardEvent("paste", { cancelable: true });
                Object.defineProperty(event, "clipboardData", { value: paste });
                dialog.dispatchEvent(event);
                const drop = new DataTransfer();
                drop.items.add(new File(["drop"], "drop.txt"));
                dialog.dispatchEvent(new DragEvent("drop", { dataTransfer: drop }));
            }"""
        )
        expect(overlay(page, "#attachments li")).to_have_count(3)
        page.get_by_role("button", name="Remove paste.txt").click()
        page.get_by_role("button", name="Remove drop.txt").click()
        expect(overlay(page, "#attachments li")).to_have_count(1)

        tap.hold.add("comment_reply")
        overlay(page, "#send").click()
        waiting(page, "the queued reply", lambda: len(tap.held) > 0)
        tap.held.clear()
        tap.hold.clear()
        _ = page.reload()
        expect(overlay(page, "#outbox-list li")).to_have_attribute(
            "data-state", "queued"
        )
        expect(overlay(page, "#badge")).to_have_text("1 queued")

        [record] = recv_all(tmp_path)
        resource = f"sha3:{sha3_256(data).hexdigest()}"
        assert record["comment"] == ""
        assert record["selector"] == "#intro"
        assert record["attachments"] == [
            {"type": "file", "file": resource, "name": "report.bin"}
        ]
        fetched = tmp_path / "fetched"
        _ = jamparty(tmp_path, "fetch", resource, "-o", str(fetched))
        assert fetched.read_bytes() == data


@pytest.mark.parametrize("selected", [False, True])
@pytest.mark.parametrize("cancelled", [False, True])
def test_html_comment_clears_highlight_and_selection(
    tmp_path: Path, page: Page, selected: bool, cancelled: bool
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        intro = page.locator("#intro")
        if selected:
            _ = intro.evaluate("""node => {
                const range = document.createRange();
                range.selectNodeContents(node);
                window.getSelection().addRange(range);
            }""")
        intro.click(button="right")
        highlight = overlay(page, "#highlight")
        expect(highlight).to_be_visible()
        preview = overlay(page, "#selection-text")
        if selected:
            expect(preview).to_contain_text("This deliberately long paragraph")
        else:
            expect(overlay(page, "#selection-preview")).to_be_hidden()
        field = overlay(page, "textarea")
        field.fill("On this paragraph")
        expect(highlight).to_be_visible()
        field.press("Escape" if cancelled else "Enter")
        expect(field).to_be_hidden()
        expect(highlight).to_be_hidden()
        assert page.evaluate("window.getSelection().rangeCount") == 0


UPLOAD = "**/_jamparty/api/attachments"


def attachment_files(tmp_path: Path, count: int) -> list[Path]:
    files = [tmp_path / f"attachment-{index}.txt" for index in range(count)]
    for path in files:
        _ = path.write_text(f"Contents of {path.name}")
    return files


def send_with_files(page: Page, files: list[Path]) -> None:
    page.locator("#intro").click(button="right")
    with page.expect_file_chooser() as chooser:
        overlay(page, "#attach").click()
    chooser.value.set_files(files)
    field = overlay(page, "textarea")
    field.fill("With files")
    field.press("Enter")
    expect(field).to_be_hidden()


def attachment_refs(files: list[Path]) -> list[dict[str, str]]:
    return [
        {
            "type": "file",
            "file": f"sha3:{sha3_256(path.read_bytes()).hexdigest()}",
            "name": path.name,
        }
        for path in files
    ]


FILE_STORE = """async (action) => {
    const db = await new Promise((resolve, reject) => {
        const request = indexedDB.open("jamparty-outbox", 2);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
    });
    try {
        return await new Promise((resolve, reject) => {
            const tx = db.transaction("files", "readwrite");
            const store = tx.objectStore("files");
            if (action === "fill" || action === "expire") {
                const count = action === "fill" ? 19 : 1;
                const at = Date.now() - (action === "expire" ? 8 * 86400000 : 0);
                for (let i = 0; i < count; i++) {
                    store.put({ at, files: [new File(["bytes"], "saved.txt")] }, "fixture-" + i);
                }
            }
            const count = store.count();
            tx.oncomplete = () => resolve(count.result);
            tx.onabort = () => reject(tx.error);
        });
    } finally {
        db.close();
    }
}"""


def test_attachment_upload_continues_after_reload(tmp_path: Path, page: Page) -> None:
    files = attachment_files(tmp_path, 4)
    uploads: list[bytes | None] = []

    def upload(route: Route) -> None:
        uploads.append(route.request.post_data_buffer)
        if len(uploads) != 4:
            route.continue_()

    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        page.route(UPLOAD, upload)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        send_with_files(page, files)
        waiting(page, "fourth upload", lambda: len(uploads) == 4)
        row = overlay(page, "#outbox-list li")
        expect(row).to_have_attribute("data-state", "sending")
        assert tap.submissions == []

        _ = page.reload()
        expect(row).to_have_attribute("data-state", "queued")
        [record] = recv_all(tmp_path)
        assert record["attachments"] == attachment_refs(files)
        assert uploads == [path.read_bytes() for path in [*files, files[-1]]]
        expect(row).to_have_attribute("data-state", "delivered")
        assert len(tap.submissions) == 1
        assert page.evaluate(FILE_STORE, "count") == 0
        resource = attachment_refs(files)[0]["file"]
        fetched = tmp_path / "fetched"
        _ = jamparty(tmp_path, "fetch", resource, "-o", str(fetched))
        assert fetched.read_bytes() == files[0].read_bytes()


@pytest.mark.parametrize("failure", [0, 503, 504])
def test_upload_retries_after_disconnect(
    tmp_path: Path, page: Page, failure: int
) -> None:
    files = attachment_files(tmp_path, 2)
    attempts = 0
    interrupted: list[Route] = []

    def upload(route: Route) -> None:
        nonlocal attempts
        attempts += 1
        if attempts == 2:
            interrupted.append(route)
        else:
            route.continue_()

    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        page.route(UPLOAD, upload)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        send_with_files(page, files)
        waiting(page, "second upload", lambda: bool(interrupted))
        ref = page.evaluate(
            "JSON.parse(sessionStorage.getItem('jamparty-outbox-v1'))[0].ref"
        )

        tap.hold.add("hello")
        tap.sockets[-1].close()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "amber")
        if failure == 0:
            interrupted[0].abort("internetdisconnected")
        else:
            interrupted[0].fulfill(status=failure, json={"error": "try again"})
        row = overlay(page, "#outbox-list li")
        expect(row).to_have_attribute("data-state", "unsent")
        assert page.evaluate(FILE_STORE, "count") == 1
        waiting(page, "reconnect hello", lambda: bool(tap.held))
        tap.release()
        expect(row).to_have_attribute("data-state", "queued")
        [record] = recv_all(tmp_path)
        assert record["attachments"] == attachment_refs(files)
        [submission] = tap.submissions
        assert submission["ref"] == ref
        assert attempts == 3


def test_duplicate_tab_recognizes_accepted_comment(
    tmp_path: Path, browser: Browser
) -> None:
    files = attachment_files(tmp_path, 1)
    uploads: list[Route] = []
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
        pages(browser, joined) as opened,
    ):
        [page] = opened
        page.route(UPLOAD, lambda route: uploads.append(route))
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        send_with_files(page, files)
        row = overlay(page, "#outbox-list li")
        expect(row).to_have_attribute("data-state", "sending")
        waiting(page, "first upload", lambda: bool(uploads))
        storage = page.evaluate("Object.entries(sessionStorage)")
        duplicate = page.context.new_page()
        try:
            # Give the second tab the same snapshot a duplicated tab gets,
            # and keep it disconnected until the first tab handles acceptance.
            other = SocketTap(duplicate)
            duplicate.add_init_script(
                f"for (const [key, value] of {json.dumps(storage)}) sessionStorage.setItem(key, value);"
            )
            uploads[0].continue_()
            expect(row).to_have_attribute("data-state", "queued")
            assert page.evaluate(FILE_STORE, "count") == 0

            _ = duplicate.goto(joined.url)
            expect(overlay(duplicate, "#outbox-list li")).to_have_attribute(
                "data-state", "queued"
            )
            expect(
                overlay(duplicate, "#outbox-list li").get_by_role("button", name="edit")
            ).to_have_count(0)
            assert other.submissions == []
            [record] = recv_all(tmp_path)
            assert record["attachments"] == attachment_refs(files)
            expect(overlay(duplicate, "#outbox-list li")).to_have_attribute(
                "data-state", "delivered"
            )
        finally:
            duplicate.close()


def test_attachment_storage_is_pruned_on_load_and_discard(
    tmp_path: Path, page: Page
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.route(UPLOAD, lambda route: route.fulfill(status=413))
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        send_with_files(page, attachment_files(tmp_path, 1))
        row = overlay(page, "#outbox-list li")
        expect(row).to_have_attribute("data-state", "failed")
        assert page.evaluate(FILE_STORE, "expire") == 2
        _ = page.reload()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        waiting(
            page,
            "expired bytes to be removed",
            lambda: page.evaluate(FILE_STORE, "count") == 1,
        )
        overlay(page, "#pill").click()
        row.get_by_role("button", name="discard").click()
        expect(row).to_have_count(0)
        waiting(
            page,
            "discarded bytes to be removed",
            lambda: page.evaluate(FILE_STORE, "count") == 0,
        )


@pytest.mark.parametrize("full", [False, True])
def test_upload_failure_can_be_edited(tmp_path: Path, page: Page, full: bool) -> None:
    path = tmp_path / "retry.txt"
    _ = path.write_text("File contents")
    upload = "**/_jamparty/api/attachments"
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.route(
            upload,
            lambda route: route.fulfill(
                status=507, json={"error": "the share's attachment storage is full"}
            ),
        )
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        page.locator("#intro").click(button="right")
        with page.expect_file_chooser() as chooser:
            overlay(page, "#attach").click()
        chooser.value.set_files(path)
        field = overlay(page, "textarea")
        field.fill("With a file")
        field.press("Enter")
        expect(field).to_be_hidden()
        expect(overlay(page, "#badge")).to_have_text("0 queued · 1 refused")
        _ = page.reload()
        overlay(page, "#pill").click()
        row = overlay(page, "#outbox-list li")
        expect(row).to_have_attribute("data-state", "failed")
        expect(row).to_contain_text(
            "retry.txt — the share's attachment storage is full"
        )
        assert tap.submissions == []
        row.get_by_role("button", name="edit").click()
        expect(field).to_have_value("With a file")
        expect(overlay(page, "#attachments .attachment-name")).to_have_text("retry.txt")
        field.fill("Corrected after refusal")
        if full:
            assert page.evaluate(FILE_STORE, "fill") == 20
        _ = page.reload()
        expect(field).to_have_value("Corrected after refusal")
        expect(overlay(page, "#attachments .attachment-name")).to_have_text("retry.txt")
        page.unroute(upload)
        field.press("Enter")
        expect(field).to_be_hidden()
        expect(row).to_have_count(1)
        expect(row).to_have_attribute("data-state", "queued")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "Corrected after refusal"
        assert record["selector"] == "#intro"
        assert record["attachments"] == [
            {
                "type": "file",
                "file": f"sha3:{sha3_256(path.read_bytes()).hexdigest()}",
                "name": "retry.txt",
            }
        ]
