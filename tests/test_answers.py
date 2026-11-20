"""The agent's answers: addressed to people, and commented on."""

import json
import subprocess
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import cast

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
        to_you = alice_page.locator("#jamparty .agent-to").first
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
