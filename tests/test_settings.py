"""Settings: the control panel, keys, the name prompt, and the settings
the page keeps for its share."""

from pathlib import Path

from playwright.sync_api import (
    Browser,
    Page,
    expect,
)
from support import (
    SITE,
    join,
    overlay,
    settings_path,
    share,
    switch,
)


def test_the_control_panel(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        # A text field of the page's own, and a record of what the page saw.
        _ = page.evaluate(
            """() => {
                const field = document.createElement("input");
                field.id = "field";
                document.querySelector("main").append(field);
                window.seen = [];
                for (const type of ["contextmenu", "keydown"]) {
                    window.addEventListener(type, (event) => {
                        window.seen.push(`${type}:${event.defaultPrevented}`);
                    });
                }
            }"""
        )
        comment_mode = switch(page, "comment-mode")
        show_cursor = switch(page, "show-cursor")

        # One panel at a time.
        overlay(page, "#gear").click()
        expect(overlay(page, "#panel")).to_be_visible()
        overlay(page, "#pill").click()
        expect(overlay(page, "#list")).to_be_visible()
        expect(overlay(page, "#panel")).to_be_hidden()
        overlay(page, "#collapse").click()
        overlay(page, "#gear").click()
        expect(overlay(page, "#panel")).to_be_visible()
        expect(overlay(page, "#panel")).to_have_css("width", "232px")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        expect(show_cursor).to_have_attribute("aria-checked", "false")
        expect(overlay(page, "#comment-mode-key")).to_have_text("C")
        expect(overlay(page, "#show-cursor-key")).to_have_text("X")

        # By click.
        comment_mode.click()
        expect(comment_mode).to_have_attribute("aria-checked", "false")
        show_cursor.click()
        expect(show_cursor).to_have_attribute("aria-checked", "true")

        # Comment mode off: right-click and / are the page's.
        _ = page.evaluate("window.seen = []")
        page.locator("#intro").click(button="right")
        page.keyboard.press("Escape")
        page.locator("h1").click()
        page.keyboard.press("/")
        expect(overlay(page, "#jampot-dialog")).not_to_be_visible()
        assert page.evaluate("window.seen") == [
            "contextmenu:false",
            "keydown:false",
            "keydown:false",
        ]

        # By key; not while a text field has focus.
        page.keyboard.press("c")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        page.keyboard.press("x")
        expect(show_cursor).to_have_attribute("aria-checked", "false")
        page.locator("#field").click()
        page.keyboard.type("cx")
        expect(page.locator("#field")).to_have_value("cx")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        expect(show_cursor).to_have_attribute("aria-checked", "false")
        page.locator("h1").click()

        # Rebinding: the new key works, the old one does not. (The clicks on
        # the page closed the panel.)
        overlay(page, "#gear").click()
        chip = overlay(page, "#comment-mode-key")
        chip.click()
        expect(chip).to_have_text("press a key…")
        expect(chip).to_have_css("border-style", "dashed")
        page.keyboard.press("k")
        expect(chip).to_have_text("K")
        page.keyboard.press("c")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        page.keyboard.press("k")
        expect(comment_mode).to_have_attribute("aria-checked", "false")
        # Escape cancels.
        chip.click()
        page.keyboard.press("Escape")
        expect(chip).to_have_text("K")
        # Keys taken by the other switch or by jampot are refused.
        chip.click()
        page.keyboard.press("x")
        expect(overlay(page, "#panel-message")).to_have_text(
            "X is taken by show my cursor"
        )
        expect(chip).to_have_text("K")
        chip.click()
        page.keyboard.press("/")
        expect(overlay(page, "#panel-message")).to_have_text("/ is used by jampot")
        expect(chip).to_have_text("K")
        expect(overlay(page, "#jampot-dialog")).not_to_be_visible()
        # The page's own keys are refused; a modifier waits for its key.
        for key in ["Enter", "Tab"]:
            chip.click()
            page.keyboard.press(key)
            expect(overlay(page, "#panel-message")).to_have_text(f"{key} is the page's")
            expect(chip).to_have_text("K")
        chip.click()
        page.keyboard.press("Shift")
        expect(chip).to_have_text("press a key…")
        page.keyboard.press("Escape")
        expect(chip).to_have_text("K")

        # Comment mode back on: / is jampot's again.
        page.keyboard.press("k")
        expect(comment_mode).to_have_attribute("aria-checked", "true")
        page.keyboard.press("/")
        expect(overlay(page, "textarea")).to_be_focused()


def test_the_name_prompt_and_settings_kept_for_the_share(
    tmp_path: Path, browser: Browser
) -> None:
    home = tmp_path / "dora"
    context = browser.new_context()
    try:
        with share(tmp_path, "serve", str(SITE)) as shared:
            with join(home, shared, name=None) as joined:
                page = context.new_page()
                _ = page.goto(joined.url)
                prompt = overlay(page, "#name-dialog")
                expect(prompt).to_be_visible()
                expect(prompt).to_contain_text("Join the potluck.")
                expect(prompt).to_contain_text(
                    "Remembered for this share in this browser; change it under the gear."
                )
                name = overlay(page, "#name-input")
                expect(name).not_to_have_value("")
                name.fill("dora")
                overlay(page, "#name-join").click()
                expect(prompt).to_be_hidden()

                # Under the gear: the name, and who is here.
                overlay(page, "#gear").click()
                expect(overlay(page, "#name-field")).to_have_value("dora")
                expect(overlay(page, "#here-count")).to_have_text("1 here")
                expect(overlay(page, ".avatar")).to_have_attribute(
                    "title", "dora (you)"
                )
                # A change updates presence.
                overlay(page, "#name-field").fill("dora.b")
                overlay(page, "#name-field").press("Enter")
                expect(overlay(page, ".avatar")).to_have_attribute(
                    "title", "dora.b (you)"
                )
                expect(overlay(page, ".avatar")).to_have_text("DB")
                # Keys and switches are kept too.
                overlay(page, "#show-cursor").click()
                expect(overlay(page, "#show-cursor")).to_have_attribute(
                    "aria-checked", "true"
                )
                overlay(page, "#comment-mode-key").click()
                page.keyboard.press("k")
                expect(overlay(page, "#comment-mode-key")).to_have_text("K")
                page.close()
                # Never on disk.
                assert not settings_path(home).exists()

            # The joiner starts over (on another port): the page hands it
            # what it keeps, and there is no prompt.
            with join(home, shared, name=None) as joined:
                page = context.new_page()
                _ = page.goto(joined.url)
                overlay(page, "#gear").click()
                expect(overlay(page, "#name-field")).to_have_value("dora.b")
                expect(overlay(page, "#show-cursor")).to_have_attribute(
                    "aria-checked", "true"
                )
                expect(overlay(page, "#comment-mode-key")).to_have_text("K")
                expect(overlay(page, ".avatar")).to_have_attribute(
                    "title", "dora.b (you)"
                )
                expect(overlay(page, "#name-dialog")).to_be_hidden()
                page.close()
                assert not settings_path(home).exists()

        # A new share is another origin: nothing kept, the prompt again.
        with (
            share(tmp_path, "serve", str(SITE)) as shared,
            join(home, shared, name=None) as joined,
        ):
            page = context.new_page()
            _ = page.goto(joined.url)
            expect(overlay(page, "#name-dialog")).to_be_visible()
            expect(overlay(page, "#name-input")).not_to_have_value("dora.b")
    finally:
        context.close()


def test_the_settings_file_gives_the_defaults(tmp_path: Path, page: Page) -> None:
    path = settings_path(tmp_path)
    path.parent.mkdir(parents=True)
    _ = path.write_text('{"name": "erin", "corner": "top-left"}')
    path.chmod(0o600)
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared, name=None) as joined,
    ):
        _ = page.goto(joined.url)
        expect(page.locator("#jampot")).to_have_attribute("data-corner", "top-left")
        expect(overlay(page, "#name-dialog")).to_be_hidden()
        overlay(page, "#gear").click()
        expect(overlay(page, "#name-field")).to_have_value("erin")
        overlay(page, '.corner[data-corner="bottom-right"]').click()
        expect(page.locator("#jampot")).to_have_attribute("data-corner", "bottom-right")
    assert path.read_text() == '{"name": "erin", "corner": "top-left"}'


# Several joiners at once: alice, bob and carol, each with their own home
# directory (key and settings), joiner process and browser context.


def test_keys_rebinding_and_text_fields(tmp_path: Path, page: Page) -> None:
    with (
        share(tmp_path, "serve", str(SITE)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = page.evaluate(
            """() => {
                const area = document.createElement("textarea");
                area.id = "area";
                const editable = document.createElement("div");
                editable.id = "editable";
                editable.contentEditable = "true";
                editable.textContent = "edit:";
                document.querySelector("main").append(area, editable);
            }"""
        )
        comment_mode = switch(page, "comment-mode")
        show_cursor = switch(page, "show-cursor")
        for field in ("#area", "#editable"):
            page.locator(field).click()
            page.keyboard.type("cx")
            expect(comment_mode).to_have_attribute("aria-checked", "true")
            expect(show_cursor).to_have_attribute("aria-checked", "false")
        expect(page.locator("#area")).to_have_value("cx")
        expect(page.locator("#editable")).to_contain_text("cx")

        # The cursor's key rebinds too.
        page.locator("h1").click()
        overlay(page, "#gear").click()
        chip = overlay(page, "#show-cursor-key")
        chip.click()
        page.keyboard.press("y")
        expect(chip).to_have_text("Y")
        page.keyboard.press("x")
        expect(show_cursor).to_have_attribute("aria-checked", "false")
        page.keyboard.press("y")
        expect(show_cursor).to_have_attribute("aria-checked", "true")

        # A held key flips once: its auto-repeats change nothing.
        _ = page.evaluate(
            """() => {
                for (let i = 0; i < 3; i++) {
                    document.body.dispatchEvent(new KeyboardEvent("keydown", {
                        key: "y", repeat: true, bubbles: true, cancelable: true,
                    }));
                }
            }"""
        )
        page.keyboard.press("c")
        expect(comment_mode).to_have_attribute("aria-checked", "false")
        expect(show_cursor).to_have_attribute("aria-checked", "true")
