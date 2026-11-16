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
        # Answers come with the first status: the draft waits for them.
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
        answer.click(button="right")
        textarea.press_sequentially("On a dismissed answer")
        message_id = answer.get_attribute("data-message-id")
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
