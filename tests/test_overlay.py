"""The overlay: the pill, edit mode, corners, and closing panels."""

import os
import signal
from pathlib import Path

from playwright.sync_api import (
    Browser,
    Locator,
    Page,
    expect,
)
from support import (
    SITE,
    jampot,
    join,
    join_process,
    overlay,
    settings_file,
    share,
    switch,
)


def test_the_pill_holds_the_dot_and_follows_the_state(
    tmp_path: Path, browser: Browser
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join_process(tmp_path, shared, shared.token) as process,
    ):
        url = process.wait_for("jampot: open ")
        context = browser.new_context()
        try:
            page = context.new_page()
            page.clock.install()
            _ = page.goto(url)
            dot = overlay(page, "#pill > #connection")
            expect(dot).to_have_attribute("data-state", "green")
            expect(dot).to_have_css("background-color", "rgb(47, 158, 68)")
            expect(overlay(page, "#pill")).to_contain_text("0 queued")
            expect(overlay(page, "#bar > #gear")).to_be_visible()

            # The joiner's event stream goes silent: the page's watchdog
            # gives up on it and reconnects, which cannot work yet.
            os.kill(process.process.pid, signal.SIGSTOP)
            try:
                page.clock.fast_forward(40_000)
                expect(dot).to_have_attribute("data-state", "amber")
                expect(dot).to_have_css("background-color", "rgb(232, 163, 61)")
            finally:
                os.kill(process.process.pid, signal.SIGCONT)
            expect(dot).to_have_attribute("data-state", "green")

            shared.process.stop()
            expect(dot).to_have_attribute("data-state", "grey")
            expect(dot).to_have_css("background-color", "rgb(154, 149, 142)")
        finally:
            context.close()


def test_edit_mode(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(
            """() => {
                window.pressed = 0;
                window.delegated = 0;
                document.querySelector("#action").addEventListener("click", () => {
                    window.pressed += 1;
                });
                document.addEventListener("click", (event) => {
                    if (event.target.closest("#action")) window.delegated += 1;
                }, true);
                const link = document.createElement("a");
                link.id = "away";
                link.href = "/elsewhere.html";
                link.textContent = "Elsewhere";
                document.querySelector("main").append(link);
            }"""
        )
        button = overlay(page, "#edit")
        outline = overlay(page, "#edit-highlight")
        dialog = overlay(page, "#jampot-dialog")
        expect(button).to_have_attribute("aria-pressed", "false")

        # On: the button turns blue; hovering outlines the element.
        button.click()
        expect(button).to_have_attribute("aria-pressed", "true")
        expect(button).to_have_css("background-color", "rgb(47, 128, 237)")
        intro = page.locator("#intro")
        intro.hover()
        expect(outline).to_be_visible()
        box = intro.bounding_box()
        drawn = outline.bounding_box()
        assert box is not None and drawn is not None
        assert abs(box["x"] - drawn["x"]) < 2 and abs(box["width"] - drawn["width"]) < 2

        # A left click comments instead of pressing the button, also for a
        # handler the page delegates from the document in the capture phase.
        page.locator("#action").click()
        expect(dialog).to_be_visible()
        expect(overlay(page, "#dialog-label")).to_have_text("Comment:")
        assert page.evaluate("() => window.pressed") == 0
        assert page.evaluate("() => window.delegated") == 0
        textarea = overlay(page, "textarea")
        textarea.fill("Make this button say what it does.")
        textarea.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        # Still in edit mode; a link does not navigate.
        expect(button).to_have_attribute("aria-pressed", "true")
        page.locator("#away").click()
        expect(dialog).to_be_visible()
        assert page.url == joined.url

        # Escape closes the dialog first, then leaves edit mode.
        page.keyboard.press("Escape")
        expect(dialog).not_to_be_visible()
        expect(button).to_have_attribute("aria-pressed", "true")
        page.keyboard.press("Escape")
        expect(button).to_have_attribute("aria-pressed", "false")
        expect(outline).to_be_hidden()

        # Off again: clicks are the page's.
        page.locator("#action").click()
        assert page.evaluate("() => window.pressed") == 1
        assert page.evaluate("() => window.delegated") == 1
        expect(dialog).not_to_be_visible()

        # The button also leaves it; the overlay works while it is on.
        button.click()
        overlay(page, "#pill").click()
        expect(overlay(page, "#list")).to_be_visible()
        expect(overlay(page, "#outbox-list li")).to_have_count(1)
        overlay(page, "#collapse").click()
        button.click()
        expect(button).to_have_attribute("aria-pressed", "false")
        page.locator("#action").click()
        assert page.evaluate("() => window.pressed") == 2


def test_edit_mode_follows_comment_mode(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(
            """() => {
                window.pressed = 0;
                document.querySelector("#action").addEventListener("click", () => {
                    window.pressed += 1;
                });
            }"""
        )
        button = overlay(page, "#edit")
        panel = overlay(page, "#panel")
        dialog = overlay(page, "#jampot-dialog")

        # In edit mode a press on the page still closes the open panel.
        overlay(page, "#gear").click()
        expect(panel).to_be_visible()
        button.click()
        expect(button).to_have_attribute("aria-pressed", "true")
        page.locator("#intro").click()
        expect(panel).to_be_hidden()
        page.keyboard.press("Escape")
        expect(dialog).not_to_be_visible()

        # Turning comment mode off leaves edit mode: clicks are the page's.
        page.keyboard.press("c")
        expect(switch(page, "comment-mode")).to_have_attribute("aria-checked", "false")
        expect(button).to_have_attribute("aria-pressed", "false")
        expect(button).to_be_disabled()
        page.locator("#action").click()
        assert page.evaluate("() => window.pressed") == 1
        expect(dialog).not_to_be_visible()
        # While it is off, edit mode cannot be entered.
        button.click(force=True)
        expect(button).to_have_attribute("aria-pressed", "false")
        page.locator("#action").click()
        assert page.evaluate("() => window.pressed") == 2

        # Comment mode on again: available again.
        page.keyboard.press("c")
        expect(button).to_be_enabled()
        button.click()
        expect(button).to_have_attribute("aria-pressed", "true")
        page.locator("#action").click()
        expect(dialog).to_be_visible()
        assert page.evaluate("() => window.pressed") == 2


def test_the_overlay_moves_to_any_corner(tmp_path: Path, page: Page) -> None:
    def place(selector: str) -> tuple[bool, bool]:
        """Whether `selector` sits in the left half and the top half."""
        size = page.viewport_size
        box = overlay(page, selector).bounding_box()
        assert size is not None and box is not None
        return (
            box["x"] + box["width"] / 2 < size["width"] / 2,
            box["y"] + box["height"] / 2 < size["height"] / 2,
        )

    def corner(name: str) -> Locator:
        return overlay(page, f'.corner[data-corner="{name}"]')

    with share(tmp_path, "serve", str(SITE)) as shared:
        with join(tmp_path, shared) as joined:
            _ = page.goto(joined.url)
            expect(overlay(page, "#connection")).to_have_attribute(
                "data-state", "green"
            )
            assert place("#bar") == (False, False)
            overlay(page, "#gear").click()
            expect(corner("bottom-right")).to_have_attribute("aria-pressed", "true")

            for name, left, top in [
                ("top-left", True, True),
                ("bottom-left", True, False),
                ("top-right", False, True),
            ]:
                if not overlay(page, "#panel").is_visible():
                    overlay(page, "#gear").click()
                corner(name).click()
                expect(corner(name)).to_have_attribute("aria-pressed", "true")
                expect(page.locator("#jampot")).to_have_attribute("data-corner", name)
                assert place("#bar") == (left, top)
                assert place("#panel") == (left, top)
                # The panel opens towards the middle: below a top bar, above
                # a bottom one.
                bar = overlay(page, "#bar").bounding_box()
                panel = overlay(page, "#panel").bounding_box()
                assert bar is not None and panel is not None
                if top:
                    assert panel["y"] >= bar["y"] + bar["height"]
                else:
                    assert panel["y"] + panel["height"] <= bar["y"]
                # The comment list goes to the same corner.
                overlay(page, "#gear").click()
                overlay(page, "#pill").click()
                expect(overlay(page, "#list")).to_be_visible()
                assert place("#list") == (left, top)
                overlay(page, "#collapse").click()

            # Top right now: the toast and unanchored answers go there too.
            _ = page.keyboard.press("/")
            textarea = overlay(page, "textarea")
            textarea.fill("A page comment.")
            textarea.press("Enter")
            expect(overlay(page, "#toast")).to_be_visible()
            assert place("#toast") == (False, True)
            _ = jampot(tmp_path, "send", "Thanks, looking at it.")
            answer = overlay(page, "#agent-general .agent-message")
            expect(answer).to_contain_text("Thanks, looking at it.")
            assert place("#agent-general .agent-message") == (False, True)

        # Saved on disk: a new joiner with the same home starts there.
        assert settings_file(tmp_path)["corner"] == "top-right"
        with join(tmp_path, shared) as joined:
            _ = page.goto(joined.url)
            expect(overlay(page, "#connection")).to_have_attribute(
                "data-state", "green"
            )
            expect(page.locator("#jampot")).to_have_attribute(
                "data-corner", "top-right"
            )
            assert place("#bar") == (False, True)


def test_outside_clicks_close_the_panel_and_the_list(
    tmp_path: Path, page: Page
) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(
            """() => {
                window.clicks = 0;
                document.querySelector("h1").addEventListener("click", () => {
                    window.clicks += 1;
                });
            }"""
        )
        panel = overlay(page, "#panel")
        gear = overlay(page, "#gear")

        # The gear still toggles; a press inside the panel keeps it open.
        gear.click()
        expect(panel).to_be_visible()
        overlay(page, "#here-count").click()
        expect(panel).to_be_visible()
        gear.click()
        expect(panel).to_be_hidden()

        # A click on the page closes the panel, and the page still gets it.
        gear.click()
        expect(panel).to_be_visible()
        page.locator("h1").click()
        expect(panel).to_be_hidden()
        assert page.evaluate("() => window.clicks") == 1

        # The same for the comment list, which has no hint text when empty.
        overlay(page, "#pill").click()
        listed = overlay(page, "#list")
        expect(listed).to_be_visible()
        expect(listed).not_to_contain_text("No comments yet")
        expect(overlay(page, "#outbox-list li")).to_have_count(0)
        page.locator("h1").click()
        expect(listed).to_be_hidden()
        expect(overlay(page, "#bar")).to_be_visible()
        assert page.evaluate("() => window.clicks") == 2
