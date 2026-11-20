"""The agent's answers: addressed to people, and commented on."""

import json
import re
import subprocess
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import cast

import pytest
from playwright.sync_api import (
    Browser,
    Page,
    expect,
)
from support import (
    BINARY,
    SITE,
    TIMEOUT,
    PageSocket,
    SocketTap,
    comment_on_intro,
    environment,
    hello,
    jamparty,
    join,
    overlay,
    pages,
    recv_all,
    settings_path,
    share,
    waiting,
)


def without_answer_timeout(home: Path) -> None:
    """Answers stay until dismissed, however slow the run."""
    path = settings_path(home)
    path.parent.mkdir(parents=True)
    path.write_text('{"answerTimeout":0}')
    path.chmod(0o600)


def test_a_draft_on_an_answer_comes_back_on_it(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(tmp_path, "send", "An answer.")
        answer = overlay(page, ".agent-message")
        expect(answer).to_contain_text("An answer.")
        answer.click(button="right")
        textarea = overlay(page, "textarea")
        textarea.press_sequentially("About the answer")
        _ = page.reload()
        # The draft still targets the reply from the snapshot without a popup.
        expect(textarea).to_have_value("About the answer")
        label = overlay(page, "#dialog-label")
        expect(label).to_have_text("Comment on agent response:")
        expect(overlay(page, "#status")).to_have_text("")
        textarea.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        [record] = recv_all(tmp_path)
        assert record["comment"] == "About the answer"
        assert str(record["selector"]).startswith("jamparty-agent:")

        # An answer dismissed meanwhile is gone: the reply goes to the page,
        # and says so, however often the page reloads.
        expect(answer).to_have_count(0)
        overlay(page, "#pill").click()
        reply = overlay(page, "#answer-list .agent-reply")
        reply.click(button="right")
        textarea.press_sequentially("On a dismissed answer")
        message_id = reply.get_attribute("data-message-id")
        assert message_id is not None
        _ = page.evaluate(
            "(id) => sessionStorage.setItem('jamparty-dismissed-v1', JSON.stringify([id]))",
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


def test_late_answers_are_quietly_listed(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        comment_on_intro(page, "First comment")
        comment_on_intro(page, "Second comment")
        expect(overlay(page, "#badge")).to_have_text("2 queued")
        first, second = recv_all(tmp_path)
        assert first["comment"] == "First comment"
        assert second["comment"] == "Second comment"
        _ = page.goto("about:blank")

        _ = jamparty(
            tmp_path, "send", "A **specific** reply.", "--anchor", str(first["id"])
        )
        _ = jamparty(tmp_path, "send", "Another reply.", "--anchor", str(second["id"]))
        _ = jamparty(tmp_path, "send", "Earlier announcement.")
        _ = jamparty(tmp_path, "send", "Later announcement.")
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        bubbles = overlay(page, ".agent-message")
        expect(bubbles).to_have_count(0)
        overlay(page, "#pill").click()
        rows = overlay(page, "#outbox-list > li")
        first_row = rows.filter(has_text="First comment")
        second_row = rows.filter(has_text="Second comment")
        expect(first_row.locator(".agent-reply strong")).to_have_text("specific")
        expect(second_row.locator(".agent-reply")).to_have_text("Another reply.")
        expect(overlay(page, "#answer-list .agent-reply")).to_have_text(
            ["Later announcement.", "Earlier announcement."]
        )
        overlay(page, "#collapse").click()

        _ = jamparty(tmp_path, "send", "Live answer.")
        expect(bubbles).to_have_count(1)
        expect(bubbles).to_contain_text("Live answer.")
        _ = page.reload()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(bubbles).to_have_count(0)
        overlay(page, "#pill").click()
        expect(overlay(page, "#answer-list .agent-reply").first).to_have_text(
            "Live answer."
        )

        _ = page.goto("about:blank")
        _ = jamparty(tmp_path, "send", "Pick one", "--choice", "Yes", "--choice", "No")
        _ = page.goto(joined.url)
        expect(bubbles).to_have_count(1)
        expect(bubbles.get_by_role("button", name="Yes")).to_be_visible()


def test_answer_catchup_on_browser_reconnect(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(tmp_path, "send", "Already shown.")
        bubbles = overlay(page, ".agent-message")
        expect(bubbles).to_have_count(1)
        tap.hold.add("answer")
        _ = jamparty(tmp_path, "send", "Missed during reconnect.")
        waiting(page, "answer for the next snapshot", lambda: bool(tap.held))
        tap.sockets[0].close()
        tap.hold.clear()
        waiting(page, "reconnected socket", lambda: len(tap.sockets) == 2)
        overlay(page, "#pill").click()
        expect(overlay(page, "#answer-list .agent-reply").first).to_have_text(
            "Missed during reconnect."
        )
        expect(bubbles).to_have_count(1)
        expect(bubbles).to_contain_text("Already shown.")
        overlay(page, "#collapse").click()
        _ = jamparty(tmp_path, "send", "Received live again.")
        expect(bubbles).to_have_count(2)
        expect(bubbles.filter(has_text="Received live again.")).to_be_visible()


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
            _ = jamparty(tmp_path, "send", "Only alice.", "--to", f"key:{fingerprint}")
            _ = jamparty(tmp_path, "send", "Everyone.")
            for page in (alice2_page, mallory_page):
                messages = page.locator("#jamparty .agent-message")
                expect(messages.filter(has_text="Everyone.")).to_have_count(1)
                expect(messages.filter(has_text="Only alice.")).to_have_count(0)
            only = alice_page.locator(
                "#jamparty .agent-message", has_text="Only alice."
            )
            expect(only.locator(".agent-to")).to_be_visible()


def test_addressed_replies(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        comment_on_intro(bob_page, "Is this right?")
        expect(bob_page.locator("#jamparty #badge")).to_have_text("1 queued")
        [line] = jamparty(tmp_path, "recv").splitlines()
        record = cast(dict[str, object], json.loads(line))

        _ = jamparty(tmp_path, "send", "Hello alice", "--to", "alice")
        _ = jamparty(tmp_path, "send", "Yes, it is.", "--anchor", str(record["id"]))
        _ = jamparty(tmp_path, "send", "Everyone: done.")

        def answers(page: Page) -> list[tuple[str, bool]]:
            messages = page.locator("#jamparty .agent-message")
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
        reply = overlay(bob_page, ".agent-placement").filter(has_text="Yes, it is.")
        expect(reply.locator(".agent-anchor-marker")).to_be_visible()
        expect(overlay(bob_page, ".agent-anchor-line")).to_have_attribute(
            "d", re.compile("C")
        )
        to_you = alice_page.locator("#jamparty .agent-to").first
        expect(to_you).to_have_text("to you")
        expect(to_you).to_have_css("color", "rgb(83, 95, 113)")
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


def test_answer_timeout_pauses_and_dismisses(tmp_path: Path, page: Page) -> None:
    path = settings_path(tmp_path)
    path.parent.mkdir(parents=True)
    path.write_text('{"answerTimeout":3600}')
    path.chmod(0o600)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        page.clock.install()
        _ = jamparty(
            tmp_path, "send", "Timed answer with [focus](https://example.org)."
        )
        answer = overlay(page, ".agent-message")
        expect(answer).to_be_visible()
        overlay(page, "#gear").click()
        # Freeze after layout, then start a fresh ten-second countdown.
        # Every assertion uses paused time, independent of startup speed.
        page.clock.pause_at(
            datetime.fromtimestamp(
                cast(int, page.evaluate("Date.now() + 10_000")) / 1000, UTC
            )
        )
        field = overlay(page, "#answer-timeout")
        field.fill("10")
        field.press("Tab")
        expect(answer.locator(".agent-timeout")).to_have_css(
            "animation-duration", "10s"
        )
        overlay(page, "#gear").dispatch_event("click")
        page.clock.run_for(3000)
        answer.hover(force=True)
        expect(answer.locator(".agent-timeout")).to_have_css(
            "animation-play-state", "paused"
        )
        page.clock.run_for(20_000)
        expect(answer).to_be_visible()
        link = answer.locator("a")
        link.focus()
        page.mouse.move(0, 0)
        page.clock.run_for(20_000)
        expect(answer).to_be_visible()
        _ = link.evaluate("node => node.blur()")
        expect(answer.locator(".agent-timeout")).to_have_css(
            "animation-play-state", "running"
        )
        page.clock.run_for(6000)
        expect(answer).to_be_visible()
        page.clock.run_for(1001)
        expect(answer).to_have_count(0)
        page.clock.resume()
        _ = page.reload()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(answer).to_have_count(0)
        assert path.read_text() == '{"answerTimeout":3600}'


def test_clear_answers_key_and_rebinding(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(tmp_path, "send", "First answer.")
        _ = jamparty(tmp_path, "send", "Second answer.")
        answers = overlay(page, ".agent-message")
        expect(answers).to_have_count(2)
        expect(overlay(page, ".agent-timeout")).to_have_count(0)
        _ = page.evaluate(
            "() => { const field = document.createElement('input'); field.id = 'typing'; document.body.append(field); }"
        )
        page.locator("#typing").fill("d")
        page.locator("#typing").press("d")
        expect(answers).to_have_count(2)
        page.locator("#typing").blur()
        page.keyboard.press("d")
        expect(answers).to_have_count(0)
        _ = jamparty(tmp_path, "send", "Third answer.")
        expect(answers).to_have_count(1)
        overlay(page, "#gear").click()
        chip = overlay(page, "#clear-answers-key")
        chip.click()
        page.keyboard.press("c")
        expect(overlay(page, "#panel-message")).to_contain_text("taken by comment mode")
        chip.click()
        page.keyboard.press("q")
        expect(chip).to_have_text("Q")
        overlay(page, "#gear").click()
        page.keyboard.press("d")
        expect(answers).to_have_count(1)
        page.keyboard.press("q")
        expect(answers).to_have_count(0)
        _ = page.reload()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(answers).to_have_count(0)


def test_timeout_panel_can_enable_and_disable(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.emulate_media(reduced_motion="reduce")
        # The page's clock stands still from the start, so the default 30 s
        # countdown cannot run out however slow the run is.
        page.clock.install()
        page.clock.pause_at(datetime.now(UTC) + timedelta(hours=1))
        with page.expect_websocket() as opened:
            _ = page.goto(joined.url)
        socket = opened.value
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        with socket.expect_event(
            "framereceived", lambda frame: "Kept answer." in str(frame)
        ):
            _ = jamparty(tmp_path, "send", "Kept answer.")
        # Let the answer's placement frame run.
        page.clock.run_for(32)
        answer = overlay(page, ".agent-message")
        expect(answer).to_be_visible()
        bar = answer.locator(".agent-timeout")
        expect(bar).to_have_css("animation-timing-function", "steps(30)")
        overlay(page, "#gear").click()
        field = overlay(page, "#answer-timeout")
        expect(field).to_have_value("30")
        field.fill("3600")
        field.press("Tab")
        expect(bar).to_be_visible()
        expect(bar).to_have_css("animation-timing-function", "steps(3600)")
        page.clock.pause_at(
            datetime.fromtimestamp(
                cast(int, page.evaluate("Date.now() + 10_000")) / 1000, UTC
            )
        )
        field.fill("0")
        field.press("Tab")
        expect(bar).to_have_count(0)
        page.clock.fast_forward(3_600_001)
        expect(answer).to_be_visible()


def test_multiple_choice_first_pick_wins(tmp_path: Path, browser: Browser) -> None:
    without_answer_timeout(tmp_path / "alice")
    without_answer_timeout(tmp_path / "bob")
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        _ = jamparty(
            tmp_path,
            "send",
            "Which heading size?",
            "--choice",
            "Keep it",
            "--choice",
            "Make it bigger",
        )
        alice_question = overlay(alice_page, ".agent-message")
        bob_question = overlay(bob_page, ".agent-message")
        expect(alice_question).to_contain_text("Which heading size?")
        expect(bob_question).to_contain_text("Which heading size?")
        answer_id = alice_question.get_attribute("data-message-id")
        assert answer_id

        # A typed reply remains an ordinary comment and keeps choices open.
        alice_question.click(button="right")
        overlay(alice_page, "textarea").fill("Could you explain?")
        overlay(alice_page, "#send").click()
        expect(overlay(alice_page, "#badge")).to_have_text("1 queued")
        [line] = jamparty(tmp_path, "recv").splitlines()
        typed = cast(dict[str, object], json.loads(line))
        assert typed["comment"] == "Could you explain?"
        assert "choice" not in typed
        expect(alice_question.locator(".agent-choices button")).to_have_count(2)

        bob_question.get_by_role("button", name="Make it bigger", exact=True).click()
        expect(bob_question).to_have_count(0)
        expect(alice_question).to_have_count(0)
        expect(overlay(bob_page, "#toast")).to_have_text("You chose: Make it bigger")
        [line] = jamparty(tmp_path, "recv").splitlines()
        picked = cast(dict[str, object], json.loads(line))
        assert picked["selector"] == f"jamparty-agent:{answer_id}"
        assert picked["comment"] == "Make it bigger"
        assert picked["choice"] == "Make it bigger"
        overlay(bob_page, "#badge").click()
        expect(overlay(bob_page, "#outbox-list li")).to_contain_text("Make it bigger")
        author = cast(dict[str, object], picked["author"])
        assert author["name"] == "bob"
        link = cast(dict[str, object], hello(bob)["link"])
        you = cast(dict[str, object], link["you"])
        assert author["key"] == you["key"]

        # A fresh joiner gets no answered question in control catch-up.
        with (
            join(tmp_path / "carol", shared, "carol") as carol,
            pages(browser, carol) as (carol_page,),
        ):
            _ = jamparty(tmp_path, "send", "Catch-up complete.")
            expect(overlay(carol_page, ".agent-message")).to_have_count(1)
            expect(overlay(carol_page, ".agent-message")).to_contain_text(
                "Catch-up complete."
            )
            expect(overlay(carol_page, ".agent-choices button")).to_have_count(0)
        _ = bob_page.reload()
        expect(overlay(bob_page, ".agent-choices button")).to_have_count(0)


def test_choice_reload_retries_saved_ref(tmp_path: Path, page: Page) -> None:
    from test_comments import HOLD_COMMENTS

    without_answer_timeout(tmp_path)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        _ = jamparty(tmp_path, "send", "Pick one", "--choice", "Yes", "--choice", "No")
        question = overlay(page, ".agent-message")
        expect(question).to_be_visible()
        _ = page.evaluate(HOLD_COMMENTS, None)
        question.get_by_role("button", name="Yes", exact=True).click()
        saved = cast(
            list[dict[str, object]],
            page.evaluate('JSON.parse(sessionStorage.getItem("jamparty-outbox-v1"))'),
        )
        [entry] = saved
        assert cast(dict[str, object], entry["draft"])["choice"] == "Yes"
        _ = page.reload()
        expect(question).to_have_count(0)
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        retried = cast(
            list[dict[str, object]],
            page.evaluate('JSON.parse(sessionStorage.getItem("jamparty-outbox-v1"))'),
        )
        assert retried[0]["ref"] == entry["ref"]
        [line] = jamparty(tmp_path, "recv").splitlines()
        assert json.loads(line)["choice"] == "Yes"


def test_late_choice_retry_is_quietly_removed(tmp_path: Path, page: Page) -> None:
    from test_comments import HOLD_COMMENTS

    without_answer_timeout(tmp_path)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        _ = jamparty(tmp_path, "send", "Pick one", "--choice", "Yes", "--choice", "No")
        question = overlay(page, ".agent-message")
        expect(question).to_be_visible()
        _ = page.evaluate(HOLD_COMMENTS, {"unsent": {"error": "disconnected"}})
        question.get_by_role("button", name="Yes", exact=True).click()
        _ = page.evaluate("window.jampartyRelease()")
        expect(overlay(page, "#toast")).to_contain_text("Choice not sent")
        # Reload retries the saved pick; the joiner's late refusal removes it.
        _ = page.add_init_script("""(() => {
            const original = WebSocket.prototype.send;
            WebSocket.prototype.send = function(data) {
                const message = JSON.parse(data);
                if (message.type !== "comment") return original.call(this, data);
                queueMicrotask(() => this.dispatchEvent(new MessageEvent("message", {
                    data: JSON.stringify({type: "comment_reply", to: message.n,
                        outcome: {refused: {error: "question is no longer available", choiceRefused: true}}})
                })));
            };
        })()""")
        _ = page.reload()
        expect(question).to_have_count(0)
        expect(overlay(page, "#outbox-list li")).to_have_count(0)
        expect(overlay(page, "#toast")).not_to_contain_text("not sent")


def test_rate_limited_choice_replay_stays_available(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        _ = jamparty(tmp_path, "send", "Pick one", "--choice", "Yes", "--choice", "No")
        question = overlay(page, ".agent-message")
        expect(question).to_be_visible()
        tap.hold.add("hello")
        tap.sockets[-1].close()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "amber")
        question.get_by_role("button", name="Yes", exact=True).click()
        expect(overlay(page, "#toast")).to_contain_text("Choice not sent")
        row = overlay(page, "#outbox-list li")
        expect(row).to_have_attribute("data-state", "unsent")
        ref = page.evaluate(
            "JSON.parse(sessionStorage.getItem('jamparty-outbox-v1'))[0].ref"
        )
        assert tap.submissions == []

        waiting(page, "reconnect hello", lambda: bool(tap.held))
        # Exhaust the share's rate limit under this joiner's key.
        other = PageSocket(joined)
        try:
            assert other.next()["type"] == "hello"
            for index in range(10):
                reply = other.request(
                    {
                        "type": "comment",
                        "ref": f"rate-limit-{index}",
                        "page": "/",
                        "selector": "#intro",
                        "comment": "Fill the rate limit",
                    }
                )
                assert "commented" in cast(dict[str, object], reply["outcome"])
        finally:
            other.close()
        tap.release()

        expect(row).to_contain_text("too many comments; try again shortly")
        expect(row).to_have_attribute("data-state", "unsent")
        expect(row).to_contain_text("Yes")
        expect(question).to_be_visible()
        overlay(page, "#pill").click()
        expect(row.get_by_role("button", name="send again")).to_be_enabled()
        [submission] = tap.submissions
        assert submission["ref"] == ref
        assert submission["choice"] == "Yes"


def test_choice_lost_ack_reloads_without_duplicate(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.add_init_script("""(() => {
            if (sessionStorage.getItem("lost-choice-ack")) return;
            const Original = WebSocket;
            window.WebSocket = class extends Original {
                constructor(...args) {
                    super(...args);
                    this.addEventListener("message", event => {
                        if (JSON.parse(event.data).type === "comment_reply") {
                            sessionStorage.setItem("lost-choice-ack", "done");
                            event.stopImmediatePropagation();
                        }
                    });
                }
            };
        })()""")
        _ = page.goto(joined.url)
        _ = jamparty(tmp_path, "send", "Pick one", "--choice", "Yes", "--choice", "No")
        question = overlay(page, ".agent-message")
        expect(question).to_be_visible()
        question.get_by_role("button", name="Yes", exact=True).click()
        expect(question).to_have_count(0)
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        saved = cast(
            list[dict[str, object]],
            page.evaluate('JSON.parse(sessionStorage.getItem("jamparty-outbox-v1"))'),
        )
        assert saved[0]["state"] == "sending"
        _ = page.reload()
        page.wait_for_function("""() => {
            const entries = JSON.parse(sessionStorage.getItem("jamparty-outbox-v1"));
            return entries[0]?.state === "queued";
        }""")
        retried = cast(
            list[dict[str, object]],
            page.evaluate('JSON.parse(sessionStorage.getItem("jamparty-outbox-v1"))'),
        )
        assert retried[0]["ref"] == saved[0]["ref"]
        [line] = jamparty(tmp_path, "recv").splitlines()
        assert json.loads(line)["choice"] == "Yes"


def test_full_outbox_keeps_question_and_reenables_choices(
    tmp_path: Path, page: Page
) -> None:
    from test_comments import FILL_OUTBOX

    without_answer_timeout(tmp_path)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        link = cast(dict[str, object], hello(joined)["link"])
        key = cast(dict[str, object], link["you"])["key"]
        _ = page.evaluate(FILL_OUTBOX, [20, key])
        tap.hold = {"comment_reply"}
        _ = page.reload()
        expect(overlay(page, "#outbox-list li")).to_have_count(20)
        _ = jamparty(tmp_path, "send", "Pick one", "--choice", "Yes", "--choice", "No")
        question = overlay(page, ".agent-message")
        yes = question.get_by_role("button", name="Yes", exact=True)
        yes.click()
        expect(overlay(page, "#toast")).to_have_text(
            "20 comments are waiting to be sent; this one can go once they have"
        )
        expect(question).to_be_visible()
        expect(yes).to_be_enabled()
        expect(question.get_by_role("button", name="No", exact=True)).to_be_enabled()
        expect(overlay(page, "#outbox-list li")).to_have_count(20)


def test_show_html_place_on_another_page(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    site = tmp_path / "site"
    site.mkdir()
    docs = site / "docs"
    docs.mkdir()
    _ = (docs / "index.html").write_text(
        '<main style="padding-top: 1600px; padding-bottom: 800px">'
        '<h1 id="intro" style="width: 350px">Introduction</h1></main>'
    )
    _ = (site / "other.html").write_text("<h1>Other page</h1>")
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(f"{joined.url.rstrip('/')}/other.html")
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path, "send", "--at", "#intro", "--page", "/docs", "See the intro."
        )
        general = overlay(page, "#agent-general .agent-message")
        expect(general).to_contain_text("See the intro.")
        general.get_by_role("link", name="Show").click()

        expect(page).to_have_url(f"{joined.url.rstrip('/')}/docs/")
        expect(page.locator("#intro")).to_be_in_viewport()
        expect(overlay(page, "#message-highlight")).to_be_visible()
        expect(overlay(page, ".agent-message")).to_have_count(0)
        overlay(page, "#pill").click()
        earlier = overlay(page, "#answer-list .agent-reply")
        expect(earlier).to_contain_text("See the intro.")
        earlier.hover()
        expect(overlay(page, "#message-highlight")).to_be_visible()
        _ = page.evaluate("window.scrollTo(0, 0)")
        expect(page.locator("#intro")).not_to_be_in_viewport()
        earlier.get_by_role("link", name="Show").click()
        expect(earlier).to_be_visible()
        expect(page.locator("#intro")).to_be_in_viewport()
        expect(overlay(page, ".agent-message")).to_have_count(0)
        overlay(page, "#collapse").click()

        _ = jamparty(
            tmp_path, "send", "--at", "#intro", "--page", "/docs/", "Another note."
        )
        answer = overlay(page, "#agent-anchored .agent-message")
        expect(answer).to_contain_text("Another note.")
        intro_box = page.locator("#intro").bounding_box()
        answer_box = answer.bounding_box()
        assert intro_box is not None and answer_box is not None
        assert abs(intro_box["y"] - answer_box["y"]) < 10
        assert answer_box["x"] >= intro_box["x"] + intro_box["width"]
        marker = overlay(page, ".agent-anchor-marker")
        leader = overlay(page, ".agent-anchor-line")
        expect(marker).to_be_visible()
        expect(leader).to_have_attribute("d", re.compile("C"))
        expect(marker).to_have_attribute("data-side", "right")
        assert (
            overlay(page, ".agent-anchor-brace").evaluate(
                "(node) => getComputedStyle(node).stroke"
            )
            == "rgb(36, 121, 233)"
        )
        expect(marker).to_have_css("background-color", "rgba(0, 0, 0, 0)")
        answer.hover()
        expect(marker).to_have_css("background-color", "rgba(47, 128, 237, 0.08)")
        page.mouse.move(0, 0)
        expect(marker).to_have_css("background-color", "rgba(0, 0, 0, 0)")
        answer.focus()
        expect(marker).to_have_css("background-color", "rgba(47, 128, 237, 0.08)")
        page.set_viewport_size({"width": 390, "height": 844})
        expect(marker).to_have_attribute("data-side", "below")
        intro_box = page.locator("#intro").bounding_box()
        answer_box = answer.bounding_box()
        assert intro_box is not None and answer_box is not None
        assert answer_box["y"] >= intro_box["y"] + intro_box["height"] + 12
        assert "system-ui" in answer.evaluate(
            "(node) => getComputedStyle(node).fontFamily"
        )
        answer.get_by_role("button", name="Dismiss agent message").click()
        expect(marker).to_have_count(0)
        expect(leader).to_have_count(0)
        _ = page.reload()
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(overlay(page, ".agent-message")).to_have_count(0)
        _ = page.goto(f"{joined.url.rstrip('/')}/other.html")
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(overlay(page, ".agent-message")).to_have_count(0)
        overlay(page, "#pill").click()
        earlier.filter(has_text="See the intro.").get_by_role(
            "link", name="Show"
        ).click()
        expect(page).to_have_url(f"{joined.url.rstrip('/')}/docs/")
        expect(page.locator("#intro")).to_be_in_viewport()
        expect(overlay(page, ".agent-message")).to_have_count(0)


def test_a_stale_show_does_not_reveal_on_another_page(
    tmp_path: Path, page: Page
) -> None:
    """A Show whose destination never loaded the overlay must not later
    scroll an unrelated page that happens to have a matching element."""
    without_answer_timeout(tmp_path)
    site = tmp_path / "site"
    site.mkdir()
    tall = (
        '<main style="padding-top: 1600px; padding-bottom: 800px">'
        '<h1 id="intro">Introduction</h1></main>'
    )
    _ = (site / "docs.html").write_text(tall)
    _ = (site / "unrelated.html").write_text(tall)
    _ = (site / "other.html").write_text("<h1>Other page</h1>")
    page.clock.install()
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        base = joined.url.rstrip("/")
        _ = page.goto(f"{base}/other.html")
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path, "send", "--at", "#intro", "--page", "/docs.html", "See it."
        )
        general = overlay(page, "#agent-general .agent-message")
        expect(general).to_contain_text("See it.")
        # The destination never loads, so its page never takes the request.
        page.route("**/docs.html", lambda route: route.abort())
        general.get_by_role("link", name="Show").click()
        page.unroute("**/docs.html")
        page.clock.fast_forward(60_000)

        _ = page.goto(f"{base}/unrelated.html")
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(page.locator("#intro")).not_to_be_in_viewport()
        expect(overlay(page, "#message-highlight")).to_be_hidden()


def test_nearby_answers_keep_separate_leaders(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    site = tmp_path / "site"
    site.mkdir()
    _ = (site / "index.html").write_text(
        '<main style="margin: 100px auto; width: 350px">'
        '<div id="first">First statement</div>'
        '<div id="second">Second statement</div></main>'
    )
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        for target in ("second", "first"):
            _ = jamparty(
                tmp_path,
                "send",
                "--at",
                f"#{target}",
                "--page",
                "/",
                f"About {target}.",
            )

        cards = []
        for target in ("first", "second"):
            group = overlay(page, ".agent-placement").filter(
                has_text=f"About {target}."
            )
            marker = group.locator(".agent-anchor-marker")
            answer_id = group.locator(".agent-message").get_attribute("data-message-id")
            leader = overlay(page, f'[data-answer-id="{answer_id}"] .agent-anchor-line')
            expect(marker).to_have_attribute("data-side", "right")
            expect(leader).to_have_attribute("d", re.compile("C"))
            assert marker.bounding_box() == page.locator(f"#{target}").bounding_box()
            card = group.locator(".agent-message").bounding_box()
            assert card is not None
            cards.append(card)
        first, second = cards
        assert first["y"] + first["height"] <= second["y"]


def test_unplaceable_question_can_be_answered(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    site = tmp_path / "site"
    site.mkdir()
    _ = (site / "index.html").write_text(
        '<main id="full" style="position:fixed; inset:0">Whole viewport</main>'
    )
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path, "send", "--at", "#full", "--page", "/", "A whole-page reply."
        )
        general = overlay(page, "#agent-general .agent-message")
        expect(general.filter(has_text="A whole-page reply.")).to_be_visible()
        _ = jamparty(
            tmp_path,
            "send",
            "--at",
            "#full",
            "--page",
            "/",
            "--choice",
            "Keep it",
            "--choice",
            "Change it",
            "Keep this layout?",
        )
        question = general.filter(has_text="Keep this layout?")
        expect(question.get_by_role("link", name="Show")).to_be_visible()
        question.get_by_role("button", name="Keep it").click()
        expect(question).to_have_count(0)
        [record] = recv_all(tmp_path)
        assert record["choice"] == "Keep it"
        assert record["comment"] == "Keep it"


def test_reply_marker_follows_parent_after_one_scroll(
    tmp_path: Path, page: Page
) -> None:
    without_answer_timeout(tmp_path)
    site = tmp_path / "site"
    site.mkdir()
    _ = (site / "index.html").write_text(
        '<main style="padding: 280px 0 1400px">'
        '<div id="target" style="width: 220px">Target</div></main>'
    )
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(tmp_path, "send", "--at", "#target", "--page", "/", "Parent note.")
        parent = overlay(page, "#agent-anchored .agent-message").filter(
            has_text="Parent note."
        )
        parent.click(button="right")
        textarea = overlay(page, "textarea")
        textarea.fill("Comment on parent")
        textarea.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        [record] = recv_all(tmp_path)
        _ = jamparty(tmp_path, "send", "--anchor", str(record["id"]), "Child reply.")
        group = overlay(page, ".agent-placement").filter(has_text="Child reply.")
        marker = group.locator(".agent-anchor-marker")
        expect(marker).to_have_attribute("data-side", "right")
        answer_id = group.locator(".agent-message").get_attribute("data-message-id")
        expect(
            overlay(page, f'[data-answer-id="{answer_id}"] .agent-anchor-line')
        ).to_have_attribute("d", re.compile("C"))
        before = parent.bounding_box()
        assert before is not None
        after = cast(
            dict[str, object],
            page.evaluate(
                """() => {
                const shadow = document.querySelector("#jamparty").shadowRoot;
                window.scrollTo(0, 120);
                // Verify the first placement pass, even if another observer
                // later schedules a second one.
                window.dispatchEvent(new Event("scroll"));
                return new Promise(resolve => requestAnimationFrame(() => {
                    const parent = [...shadow.querySelectorAll(".agent-message")]
                        .find(node => node.textContent.includes("Parent note."));
                    const child = [...shadow.querySelectorAll(".agent-placement")]
                        .find(node => node.textContent.includes("Child reply."));
                    const marker = child.querySelector(".agent-anchor-marker");
                    const bounds = parent.getBoundingClientRect();
                    const id = child.querySelector(".agent-message").dataset.messageId;
                    const leader = shadow.querySelector(`[data-answer-id="${id}"] .agent-anchor-line`);
                    const start = leader.getPointAtLength(0).matrixTransform(leader.getScreenCTM());
                    resolve({
                        parent: bounds.toJSON(),
                        marker: marker.getBoundingClientRect().toJSON(),
                        leaderAttached: start.x > bounds.right
                            && Math.abs(start.y - (bounds.top + bounds.bottom) / 2) < .1,
                    });
                }));
            }"""
            ),
        )
        assert after["parent"] == after["marker"]
        assert after["leaderAttached"] is True
        assert cast(dict[str, float], after["parent"])["y"] < before["y"]


def test_clipped_answer_keeps_internal_scroll(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    site = tmp_path / "site"
    site.mkdir()
    _ = (site / "index.html").write_text(
        '<main style="padding: 180px 0 1400px">'
        '<div id="target" style="width: 220px">Target</div></main>'
    )
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        message = "\n\n".join(
            f"Paragraph {index} for the long answer." for index in range(100)
        )
        _ = jamparty(tmp_path, "send", "--at", "#target", "--page", "/", message)
        answer = overlay(page, "#agent-anchored .agent-message")
        expect(answer).to_be_visible()
        _ = answer.evaluate("(node) => { node.scrollTop = 200; }")
        _ = page.evaluate("window.scrollTo(0, 80)")
        assert (
            overlay(page, ".agent-anchor-marker").bounding_box()
            == page.locator("#target").bounding_box()
        )
        assert answer.evaluate("(node) => node.scrollTop") == 200
        page.set_viewport_size({"width": 540, "height": 620})
        expect(answer).not_to_have_css("width", "340px")
        assert answer.evaluate("(node) => node.scrollTop") == 200


def test_fixed_targets_follow_their_scroll_sources(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "index.html"
    _ = path.write_text("""
        <body style="min-height: 1800px">
          <section id="panel" style="position: fixed; top: 50px; left: 80px;
            width: 260px; height: 260px; overflow: auto">
            <div id="panel-target" style="margin: 100px 0 500px">Panel target</div>
          </section>
          <section style="transform: translateX(0); margin-top: 420px">
            <div id="transformed-target" style="position: fixed; left: 80px;
              width: 240px">Transformed target</div>
          </section>
        </body>
    """)
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        for target in ("panel-target", "transformed-target"):
            _ = jamparty(
                tmp_path, "send", "--at", f"#{target}", "--page", "/index.html", target
            )
            group = overlay(page, ".agent-placement").filter(has_text=target)
            brace = group.locator(".agent-anchor-brace")
            expect(brace).to_have_attribute("d", re.compile("^M"))
            delta, aligned = cast(
                list[float],
                brace.evaluate(
                    """async (brace, target) => {
                        const anchor = document.getElementById(target);
                        const marker = brace.closest(".agent-placement")
                            .querySelector(".agent-anchor-marker");
                        const edgeBefore = brace.getPointAtLength(0)
                            .matrixTransform(brace.getScreenCTM()).y;
                        await new Promise(resolve => {
                            const source = target === "panel-target"
                                ? document.querySelector("#panel") : document;
                            window.addEventListener("scroll", event => {
                                if (event.target === source) requestAnimationFrame(resolve);
                            }, { capture: true, once: true });
                            if (source === document) window.scrollTo(0, 80);
                            else source.scrollTop = 60;
                        });
                        const after = anchor.getBoundingClientRect();
                        const edgeAfter = brace.getPointAtLength(0)
                            .matrixTransform(brace.getScreenCTM()).y;
                        return [edgeAfter - edgeBefore,
                                marker.getBoundingClientRect().top - after.top];
                    }""",
                    target,
                ),
            )
            assert delta == (-60 if target == "panel-target" else -80)
            assert aligned == 0


@pytest.mark.parametrize("root_overflow", ["hidden", "clip"])
def test_a_scrolling_body_moves_card_and_highlight(
    tmp_path: Path, page: Page, root_overflow: str
) -> None:
    """With the root clipped, the body is an element scroller, not the viewport."""
    without_answer_timeout(tmp_path)
    path = tmp_path / "index.html"
    _ = path.write_text(f"""
        <html style="overflow: {root_overflow}; height: 100%">
          <body style="margin: 0; height: 100vh; overflow: auto">
            <div style="height: 300px"></div>
            <div id="target" style="margin-left: 400px; width: 200px">Target</div>
            <div style="height: 2000px"></div>
          </body>
        </html>
    """)
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path, "send", "--at", "#target", "--page", "/index.html", "Here"
        )
        group = overlay(page, ".agent-placement").filter(has_text="Here")
        expect(group.locator(".agent-anchor-brace")).to_have_attribute(
            "d", re.compile("^M")
        )
        # Enter on a focused anchored answer reveals its target with the outline.
        message = group.locator(".agent-message")
        message.focus()
        expect(message).to_be_focused()
        page.keyboard.press("Enter")
        expect(overlay(page, "#message-highlight")).to_be_visible()
        offsets = cast(
            list[float],
            page.evaluate(
                """async () => {
                    const root = document.querySelector("#jamparty").shadowRoot;
                    const target = document.getElementById("target");
                    await new Promise(resolve => {
                        document.body.addEventListener("scroll", () =>
                            requestAnimationFrame(() => requestAnimationFrame(resolve)),
                            { once: true });
                        document.body.scrollTop = 120;
                    });
                    const top = target.getBoundingClientRect().top;
                    const marker = root.querySelector(".agent-anchor-marker");
                    const highlight = root.querySelector("#message-highlight");
                    return [marker.getBoundingClientRect().top - top,
                            highlight.getBoundingClientRect().top - top];
                }"""
            ),
        )
        assert offsets == [0, 0], offsets


def test_anchored_answer_reappears_at_viewport_edge(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "index.html"
    _ = path.write_text(
        '<main style="height:2000px">'
        '<div id="target" style="position:absolute;top:300px;left:400px;'
        'width:170px;height:40px">Target</div></main>'
    )
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path, "send", "--at", "#target", "--page", "/index.html", "A reply."
        )
        card = overlay(page, "#agent-anchored .agent-message")
        expect(card).to_be_visible()
        _ = page.evaluate("window.scrollTo(0, 400)")
        expect(card).to_be_hidden()
        _ = page.locator("#target").evaluate("""target => new Promise((resolve, reject) => {
            const observer = new IntersectionObserver(([entry]) => {
                if (!entry.isIntersecting || entry.intersectionRect.height !== 0) return;
                clearTimeout(timeout);
                observer.disconnect();
                requestAnimationFrame(() => requestAnimationFrame(resolve));
            });
            const timeout = setTimeout(() => {
                observer.disconnect();
                reject(new Error("Target did not reach exact edge contact"));
            }, 5000);
            observer.observe(target);
            window.scrollTo(0, 340);
        })""")
        _ = page.evaluate("window.scrollTo(0, 240)")
        expect(page.locator("#target")).to_be_in_viewport(ratio=1)
        expect(card).to_be_visible()


def test_new_anchor_repositions_cached_answer(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "index.html"
    _ = path.write_text("""
        <div id="first" style="position:absolute;top:240px;left:420px;width:170px">
            First
        </div>
        <div id="second" style="position:absolute;top:245px;left:650px;width:100px">
            Second
        </div>
    """)
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path, "send", "--at", "#first", "--page", "/index.html", "First reply."
        )
        first = overlay(page, ".agent-placement").filter(has_text="First reply.")
        marker = first.locator(".agent-anchor-marker")
        expect(marker).to_have_attribute("data-side", "right")
        old_card = first.locator(".agent-message").bounding_box()
        second = page.locator("#second").bounding_box()
        assert old_card is not None and second is not None
        assert old_card["x"] < second["x"] < old_card["x"] + old_card["width"]
        assert old_card["y"] < second["y"] < old_card["y"] + old_card["height"]
        _ = jamparty(
            tmp_path,
            "send",
            "--at",
            "#second",
            "--page",
            "/index.html",
            "Second reply.",
        )
        cards = overlay(page, "#agent-anchored .agent-message")
        expect(cards).to_have_count(2)
        expect(marker).to_have_attribute("data-side", "left")
        assert cards.evaluate_all("""cards => {
            const targets = [...document.querySelectorAll("#first, #second")];
            return cards.every(card => {
                const a = card.getBoundingClientRect();
                return targets.every(target => {
                    const b = target.getBoundingClientRect();
                    return a.right <= b.left || a.left >= b.right ||
                        a.bottom <= b.top || a.top >= b.bottom;
                });
            });
        }""")


def test_open_comment_list_repositions_anchored_answer(
    tmp_path: Path, page: Page
) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "index.html"
    _ = path.write_text(
        '<div id="target" style="position:absolute;top:610px;left:510px;width:170px">'
        "Target</div>"
    )
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path, "send", "--at", "#target", "--page", "/index.html", "A reply."
        )
        marker = overlay(page, ".agent-anchor-marker")
        expect(marker).to_have_attribute("data-side", "right")
        old_card = overlay(page, "#agent-anchored .agent-message").bounding_box()
        assert old_card is not None
        overlay(page, "#pill").click()
        comment_list = overlay(page, "#list").bounding_box()
        assert comment_list is not None
        assert old_card["x"] + old_card["width"] > comment_list["x"]
        assert old_card["y"] + old_card["height"] > comment_list["y"]
        expect(marker).to_have_attribute("data-side", "left")


def test_moving_first_answer_keeps_grip_reachable(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "index.html"
    _ = path.write_text(
        '<main style="min-height:1800px;padding-top:65px">'
        '<div id="target" style="width:200px">First line</div></main>'
    )
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path, "send", "--at", "#target", "--page", "/index.html", "A reply."
        )
        card = overlay(page, "#agent-anchored .agent-message")
        heading = card.locator(".agent-header")
        grip = card.get_by_role("button", name="Move")
        expect(heading).to_be_visible()
        box = heading.bounding_box()
        assert box is not None
        x = box["x"] + box["width"] / 2
        page.mouse.move(x, box["y"] + box["height"] / 2)
        page.mouse.down()
        page.mouse.move(x, 0, steps=8)
        page.mouse.up()
        expect(card).to_have_css("top", "8px")
        expect(card).to_be_in_viewport(ratio=1)
        expect(grip).to_be_in_viewport(ratio=1)
        grip.focus()
        for _ in range(8):
            page.keyboard.press("Shift+ArrowUp")
        _ = page.evaluate("window.scrollTo(0, 0)")
        expect(card).to_be_in_viewport(ratio=1)
        expect(grip).to_be_in_viewport(ratio=1)
        before = card.bounding_box()
        assert before is not None
        page.keyboard.press("ArrowDown")
        _ = page.wait_for_function(
            """y => Math.abs(document.querySelector("#jamparty").shadowRoot
                .querySelector("#agent-anchored .agent-message")
                .getBoundingClientRect().top - y - 16) < 1""",
            arg=before["y"],
        )


def test_nested_scroll_batches_affected_card_reads(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "index.html"
    _ = path.write_text("""
        <section style="height:360px;overflow:auto">
          <section id="scroller" style="height:300px;overflow:auto;margin-left:80px">
            <div id="first" style="margin-top:100px;width:240px">First</div>
            <div id="second" style="margin:100px 0 500px;width:240px">Second</div>
          </section>
        </section>
        <section style="height:200px;overflow:auto;margin-left:80px">
          <div id="independent" style="margin:50px 0 500px;width:240px">Other</div>
        </section>
        <section id="unannotated" style="height:100px;overflow:auto">
          <div style="height:300px">No annotations</div>
        </section>
    """)
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        for target in ("first", "second", "independent"):
            _ = jamparty(
                tmp_path, "send", "--at", f"#{target}", "--page", "/index.html", target
            )
        expect(overlay(page, ".agent-placement")).to_have_count(3)
        card = overlay(page, ".agent-message").filter(has_text="first")
        expect(card).to_be_visible()
        card.focus()
        expect(card).to_be_focused()
        page.keyboard.press("Enter")
        highlight = overlay(page, "#message-highlight")
        expect(highlight).to_be_visible()
        frames = cast(
            list[list[str]],
            page.evaluate("""async () => {
                const shadow = document.querySelector("#jamparty").shadowRoot;
                const scroller = document.querySelector("#scroller");
                await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
                const wrappers = [...shadow.querySelectorAll(".agent-placement")];
                const styles = new Map(wrappers.map(node => [
                    node.style, node.querySelector(".agent-body").textContent.trim()
                ]));
                const measure = Element.prototype.getBoundingClientRect;
                const set = CSSStyleDeclaration.prototype.setProperty;
                const frames = [];
                let calls = [];
                Element.prototype.getBoundingClientRect = function () {
                    calls.push("read:" + this.id);
                    return measure.call(this);
                };
                CSSStyleDeclaration.prototype.setProperty = function (name, value, ...rest) {
                    if (name === "transform" && styles.has(this)) {
                        calls.push("write:" + styles.get(this));
                    }
                    return set.call(this, name, value, ...rest);
                };
                try {
                    const independent = document.querySelector("#independent").parentElement;
                    const unannotated = document.querySelector("#unannotated");
                    for (const [source, top] of [[scroller, 20], [scroller, 40], [scroller, 60],
                                                 [independent, 10], [unannotated, 10]]) {
                        calls = [];
                        await new Promise(resolve => {
                            source.addEventListener("scroll", () => {
                                requestAnimationFrame(resolve);
                            }, { once: true });
                            source.scrollTop = top;
                        });
                        frames.push(calls);
                    }
                    return frames;
                } finally {
                    Element.prototype.getBoundingClientRect = measure;
                    CSSStyleDeclaration.prototype.setProperty = set;
                }
            }"""),
        )
        expect(highlight).to_be_visible()
        assert frames[-2:] == [["read:independent", "write:independent"], []]
        for calls in frames[:-2]:
            assert set(calls[:2]) == {"read:first", "read:second"}
            assert len(calls) == 4
            assert all(call.startswith("write:") for call in calls[2:])
            assert not any("independent" in call for call in calls)
