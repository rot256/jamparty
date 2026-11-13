"""The agent's answers: addressed to people, and commented on."""

import json
import subprocess
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
    jampot,
    join,
    overlay,
    pages,
    recv_all,
    share,
)


def test_a_draft_on_an_answer_comes_back_on_it(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jampot(tmp_path, "send", "An answer.")
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
        assert str(record["selector"]).startswith("jampot-agent:")

        # An answer dismissed meanwhile is gone: the reply goes to the page,
        # and says so, however often the page reloads.
        answer.click(button="right")
        textarea.press_sequentially("On a dismissed answer")
        message_id = answer.get_attribute("data-message-id")
        assert message_id is not None
        _ = page.evaluate(
            "(id) => sessionStorage.setItem('jampot-dismissed-v1', JSON.stringify([id]))",
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
            _ = jampot(tmp_path, "send", "Only alice.", "--to", f"key:{fingerprint}")
            _ = jampot(tmp_path, "send", "Everyone.")
            for page in (alice2_page, mallory_page):
                messages = page.locator("#jampot .agent-message")
                expect(messages.filter(has_text="Everyone.")).to_have_count(1)
                expect(messages.filter(has_text="Only alice.")).to_have_count(0)
            only = alice_page.locator("#jampot .agent-message", has_text="Only alice.")
            expect(only.locator(".agent-to")).to_be_visible()


def test_addressed_replies(tmp_path: Path, browser: Browser) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path / "alice", shared, "alice") as alice,
        join(tmp_path / "bob", shared, "bob") as bob,
        pages(browser, alice, bob) as (alice_page, bob_page),
    ):
        comment_on_intro(bob_page, "Is this right?")
        expect(bob_page.locator("#jampot #badge")).to_have_text("1 queued")
        [line] = jampot(tmp_path, "recv").splitlines()
        record = cast(dict[str, object], json.loads(line))

        _ = jampot(tmp_path, "send", "Hello alice", "--to", "alice")
        _ = jampot(tmp_path, "send", "Yes, it is.", "--anchor", str(record["id"]))
        _ = jampot(tmp_path, "send", "Everyone: done.")

        def answers(page: Page) -> list[tuple[str, bool]]:
            messages = page.locator("#jampot .agent-message")
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
        to_you = alice_page.locator("#jampot .agent-to").first
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
