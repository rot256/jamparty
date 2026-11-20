"""Theme, drag and connector geometry in both overlay entry points."""

import re
from pathlib import Path
from typing import cast

import pytest
from playwright.sync_api import Locator, Page, expect
from support import jamparty, join, overlay, recv_all, share
from test_answers import without_answer_timeout
from test_pdf import fixture_pdf


def start_drag(page: Page, handle: Locator, dx: int, dy: int) -> None:
    box = handle.bounding_box()
    assert box is not None
    x = box["x"] + box["width"] / 3
    y = box["y"] + box["height"] / 2
    page.mouse.move(x, y)
    page.mouse.down()
    page.mouse.move(x + dx, y + dy, steps=8)


def drag(page: Page, handle: Locator, dx: int, dy: int) -> None:
    start_drag(page, handle, dx, dy)
    page.mouse.up()


def expect_position(surface: Locator, x: float, y: float) -> None:
    node = surface.element_handle()
    try:
        _ = surface.page.wait_for_function(
            """({ node, x, y }) => {
                const rect = node.getBoundingClientRect();
                return Math.abs(rect.x - x) < 1 && Math.abs(rect.y - y) < 1;
            }""",
            arg={"node": node, "x": x, "y": y},
        )
    finally:
        node.dispose()


def cancel_drag(
    page: Page, surface: Locator, handle: Locator, dx: int, dy: int
) -> None:
    start = surface.bounding_box()
    assert start is not None
    capture_handle = (
        handle.locator("..") if handle.get_attribute("id") == "dialog-label" else handle
    )
    _ = capture_handle.evaluate("""node => {
        node.addEventListener("pointerdown", event => {
            node.dataset.testPointer = event.pointerId;
        }, { once: true });
    }""")
    start_drag(page, handle, dx, dy)
    expect_position(surface, start["x"] + dx, start["y"] + dy)
    capture = "node => node.hasPointerCapture(Number(node.dataset.testPointer))"
    assert capture_handle.evaluate(capture)
    page.keyboard.press("Escape")
    expect_position(surface, start["x"], start["y"])
    assert not capture_handle.evaluate(capture)
    page.mouse.move(10, 10)
    page.mouse.up()
    expect_position(surface, start["x"], start["y"])


def move_with_keys(surface: Locator, next_focus: Locator) -> None:
    move = surface.get_by_role("button", name="Move", exact=True)
    expect(move).to_be_focused()
    expect(move).to_have_css("outline-width", "2px")
    start = surface.bounding_box()
    assert start is not None
    x, y = start["x"], start["y"]
    move.press("ArrowLeft")
    expect_position(surface, x - 16, y)
    move.press("Shift+ArrowUp")
    expect_position(surface, x - 16, y - 64)
    move.press("Escape")
    expect_position(surface, x, y)
    move.press("ArrowLeft")
    move.press("Enter")
    expect_position(surface, x - 16, y)
    move.press("ArrowLeft")
    move.press("Escape")
    expect_position(surface, x - 16, y)
    move.press("ArrowUp")
    next_focus.focus()
    move.focus()
    move.press("ArrowLeft")
    move.press("Escape")
    expect_position(surface, x - 16, y - 16)


def test_theme_and_dialog_on_serif_page(tmp_path: Path, page: Page) -> None:
    path = tmp_path / "index.html"
    _ = path.write_text(
        '<html style="font: 24px Georgia, serif"><h1>Review copy</h1></html>'
    )
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.emulate_media(color_scheme="dark")
        _ = page.goto(joined.url)
        host = page.locator("#jamparty")
        expect(host).to_have_attribute("data-theme", "auto")
        pill = overlay(page, "#pill")
        expect(pill).to_have_css("background-color", "rgb(22, 22, 22)")
        page.emulate_media(color_scheme="light")
        expect(pill).to_have_css("background-color", "rgb(255, 255, 255)")

        gear = overlay(page, "#gear")
        gear.click()
        dark = overlay(page, "button[data-theme=dark]")
        dark.click()
        expect(dark).to_have_attribute("aria-pressed", "true")
        gear.click()
        page.mouse.move(0, 0)
        surfaces = (
            "#pill",
            "#gear",
            "#edit",
            "#panel",
            "#list",
            "#toast",
            "#name-dialog",
            "#jamparty-dialog",
        )
        computed = """node => {
            const { color, backgroundColor, fontFamily } = getComputedStyle(node);
            return [color, backgroundColor, fontFamily];
        }"""
        baseline = [overlay(page, selector).evaluate(computed) for selector in surfaces]
        for _, background, font in baseline:
            assert background == "rgb(22, 22, 22)"
            assert font.startswith("system-ui")
        _ = page.add_style_tag(
            content="""
            #jamparty {
                --jp-text: transparent;
                --jp-surface: lime;
                --jp-font: monospace;
                font: 32px Georgia, serif;
            }
        """
        )
        expect(host).to_have_css("font-size", "32px")
        assert [
            overlay(page, selector).evaluate(computed) for selector in surfaces
        ] == baseline
        _ = page.reload()
        expect(host).to_have_attribute("data-theme", "dark")
        expect(pill).to_have_css("background-color", "rgb(22, 22, 22)")
        gear.click()
        overlay(page, "button[data-theme=light]").click()
        expect(host).to_have_attribute("data-theme", "light")
        gear.click()
        page.mouse.move(0, 0)
        for selector in surfaces:
            expect(overlay(page, selector)).to_have_css(
                "background-color", "rgb(255, 255, 255)"
            )

        _ = jamparty(tmp_path, "send", "A new answer.")
        answer = overlay(page, ".agent-message")
        header = answer.locator(".agent-header")
        grip = header.get_by_role("button", name="Move", exact=True)
        expect(header.locator(":scope > :first-child")).to_have_attribute(
            "aria-label", "Move"
        )
        expect(grip).to_have_css("width", "21px")
        expect(header).to_have_css("background-color", "rgb(240, 243, 247)")
        expect(answer).to_have_css("background-color", "rgb(255, 255, 255)")
        gear.click()
        overlay(page, "button[data-theme=dark]").click()
        expect(answer).to_have_css("background-color", "rgb(22, 22, 22)")
        expect(header).to_have_css("background-color", "rgb(36, 39, 45)")
        gear.click()

        page.keyboard.press("/")
        field = overlay(page, "textarea")
        expect(field).to_be_focused()
        field.fill("A draft whose position I can change.")
        assert field.evaluate("(node) => getComputedStyle(node).fontFamily").startswith(
            "system-ui"
        )
        expect(field).to_have_css("box-shadow", "none")
        expect(field).to_have_css("outline-style", "solid")
        expect(field).to_have_css("outline-width", "2px")
        expect(field).to_have_css("border-top-color", "rgba(0, 0, 0, 0)")
        dialog = overlay(page, "#jamparty-dialog")
        title_bar = dialog.locator("#dialog-header")
        expect(title_bar).to_have_css("background-color", "rgb(36, 39, 45)")
        expect(title_bar.locator(":scope > :first-child")).to_have_attribute(
            "aria-label", "Move"
        )
        cancel_drag(page, dialog, dialog.locator("#dialog-label"), -120, -80)
        expect(dialog).to_be_visible()
        expect(field).to_have_value("A draft whose position I can change.")
        field.press("Shift+Tab")
        move_with_keys(dialog, field)
        field.focus()
        before = dialog.bounding_box()
        drag(page, overlay(page, "#dialog-label"), -170, -100)
        after = dialog.bounding_box()
        assert before is not None and after is not None
        assert after["x"] < before["x"] - 100
        assert after["y"] < before["y"] - 70
        expect(field).to_be_focused()
        expect(field).to_have_value("A draft whose position I can change.")


def test_pdf_margin_and_dragged_connector(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "paper.pdf"
    _ = path.write_bytes(fixture_pdf("Second page", outline_child="Appendix"))
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.set_viewport_size({"width": 1440, "height": 900})
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(page.locator(".pdf-page").first).to_have_attribute(
            "data-rendered", "true"
        )
        gear = overlay(page, "#gear")
        gear.click()
        overlay(page, "button[data-theme=dark]").click()
        expect(page.locator("#jamparty")).to_have_attribute("data-theme", "dark")
        gear.click()
        sections = overlay(page, "#pdf-outline")
        sections.click()
        expect(overlay(page, "#pdf-sections")).to_have_css(
            "background-color", "rgb(22, 22, 22)"
        )
        sections.click()
        page.mouse.move(0, 0)
        expect(sections).to_have_css("background-color", "rgb(22, 22, 22)")
        _ = jamparty(
            tmp_path,
            "send",
            "--at-pdf",
            "paper.pdf#page=1&box=72,515,540,545",
            "Check the proof.",
        )
        answer = overlay(page, ".agent-message")
        expect(answer).to_be_visible()
        mark = page.locator(".pdf-answer-anchor")
        heading = overlay(page, ".agent-header")
        target_box = mark.bounding_box()
        start = answer.bounding_box()
        assert target_box is not None and start is not None
        assert start["y"] == pytest.approx(target_box["y"], abs=1)
        assert start["x"] + start["width"] < 240  # The bar occupies the right margin.
        brace = overlay(page, ".agent-anchor-brace")
        link = overlay(page, ".agent-anchor-line")

        def check_link() -> None:
            bounds = mark.bounding_box()
            card = answer.bounding_box()
            assert bounds is not None and card is not None
            brace_box = cast(
                list[float],
                brace.evaluate(
                    """p => {
                        const b = p.getBBox();
                        const m = p.getScreenCTM();
                        const start = new DOMPoint(b.x, b.y).matrixTransform(m);
                        const end = new DOMPoint(b.x + b.width, b.y + b.height).matrixTransform(m);
                        return [start.x, start.y, end.x - start.x, end.y - start.y];
                    }"""
                ),
            )
            assert brace_box[1] == pytest.approx(bounds["y"], abs=0.1)
            assert brace_box[3] == pytest.approx(bounds["height"], abs=0.1)
            assert brace_box[0] + brace_box[2] < bounds["x"]
            endpoints = cast(
                list[float],
                link.evaluate("""p => {
                    const matrix = p.getScreenCTM();
                    const start = p.getPointAtLength(0).matrixTransform(matrix);
                    const end = p.getPointAtLength(p.getTotalLength()).matrixTransform(matrix);
                    return [start.x, start.y, end.x, end.y];
                }"""),
            )
            assert endpoints[0] == pytest.approx(brace_box[0], abs=0.1)
            assert endpoints[1] == pytest.approx(
                bounds["y"] + bounds["height"] / 2, abs=0.1
            )
            assert endpoints[2] == pytest.approx(card["x"] + card["width"], abs=0.1)
            assert card["y"] < endpoints[3] < card["y"] + card["height"]

        check_link()
        original = link.get_attribute("d")
        reads = page.evaluate("""async () => {
            const measure = Element.prototype.getBoundingClientRect;
            let reads = 0;
            Element.prototype.getBoundingClientRect = function () {
                if (this.getRootNode() instanceof ShadowRoot ||
                    this.classList.contains("pdf-answer-anchor")) reads++;
                return measure.call(this);
            };
            try {
                window.scrollBy(0, 400);
                for (let i = 0; i < 12; i++) window.dispatchEvent(new Event("scroll"));
                await new Promise(resolve => requestAnimationFrame(
                    () => requestAnimationFrame(resolve)));
                return reads;
            } finally {
                Element.prototype.getBoundingClientRect = measure;
            }
        }""")
        assert reads == 0
        expect_position(answer, start["x"], start["y"] - 400)
        expect_position(mark, target_box["x"], target_box["y"] - 400)
        expect(link).to_have_attribute("d", original or "")
        check_link()
        _ = page.evaluate("window.scrollTo(0, 0)")
        expect_position(answer, start["x"], start["y"])
        cancel_drag(page, answer, heading, 0, -80)
        expect(link).to_have_attribute("d", original or "")
        expect(answer).to_have_class("agent-message has-anchor")
        drag(page, heading, 0, -130)
        expect(link).not_to_have_attribute("d", original or "")
        moved = answer.bounding_box()
        assert moved is not None
        assert moved["y"] == pytest.approx(start["y"] - 130, abs=1)
        check_link()
        manual = link.get_attribute("d")
        _ = page.evaluate("window.scrollBy(0, 75)")
        expect_position(answer, moved["x"], moved["y"] - 75)
        expect(link).to_have_attribute("d", manual or "")
        check_link()
        page.set_viewport_size({"width": 1520, "height": 960})
        resized = mark.bounding_box()
        assert resized is not None
        expect_position(
            answer,
            moved["x"] + resized["x"] - target_box["x"],
            moved["y"] + resized["y"] - target_box["y"],
        )
        check_link()
        _ = page.evaluate("window.scrollBy(0, 1100)")
        expect(mark).not_to_be_in_viewport()
        expect(answer).not_to_be_in_viewport()
        _ = page.evaluate("window.scrollTo(0, 75)")
        expect(answer).to_be_in_viewport()
        check_link()


def test_pdf_drag_measures_card_once_per_frame(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "paper.pdf"
    _ = path.write_bytes(fixture_pdf("Second page"))
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.set_viewport_size({"width": 1440, "height": 900})
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(page.locator(".pdf-page").first).to_have_attribute(
            "data-rendered", "true"
        )
        _ = jamparty(
            tmp_path,
            "send",
            "--at-pdf",
            "paper.pdf#page=1&box=72,515,540,545",
            "Check the proof.",
        )
        answer = overlay(page, ".agent-message")
        heading = answer.locator(".agent-header")
        expect(overlay(page, ".agent-anchor-line")).to_have_attribute(
            "d", re.compile("^M")
        )
        start = answer.bounding_box()
        assert start is not None
        _ = heading.evaluate("""node => {
            node.addEventListener("pointerdown", event => {
                node.dataset.testPointer = event.pointerId;
            }, { once: true });
        }""")
        # A real pointerdown establishes capture before the synthetic burst.
        start_drag(page, heading, 0, 0)
        try:
            synchronous, per_frame, x, y, end_x, end_y = cast(
                list[float],
                heading.evaluate("""async heading => {
                    const card = heading.closest(".agent-message");
                    const link = heading.getRootNode().querySelector(".agent-anchor-line");
                    const measure = Element.prototype.getBoundingClientRect;
                    const handle = measure.call(heading);
                    let measurements = 0;
                    Element.prototype.getBoundingClientRect = function () {
                        if (this === card) measurements++;
                        return measure.call(this);
                    };
                    try {
                        for (let step = 1; step <= 12; step++) {
                            heading.dispatchEvent(new PointerEvent("pointermove", {
                                pointerId: Number(heading.dataset.testPointer),
                                pointerType: "mouse",
                                isPrimary: true,
                                buttons: 1,
                                clientX: handle.x + handle.width / 3,
                                clientY: handle.y + handle.height / 2 - step * 10,
                                bubbles: true,
                            }));
                        }
                        const synchronous = measurements;
                        await new Promise(resolve => requestAnimationFrame(resolve));
                        // Geometry assertions must not count as placement reads.
                        const rect = measure.call(card);
                        const end = link.getPointAtLength(link.getTotalLength());
                        return [synchronous, measurements - synchronous,
                                rect.x, rect.y, end.x, end.y];
                    } finally {
                        Element.prototype.getBoundingClientRect = measure;
                    }
                }"""),
            )
        finally:
            page.mouse.up()
        assert synchronous == 0
        assert per_frame == 1
        assert x == pytest.approx(start["x"], abs=1)
        assert y == pytest.approx(start["y"] - 120, abs=1)
        assert end_x == pytest.approx(start["x"] + start["width"], abs=1)
        # The target is below the moved card, so the endpoint sits at the lower inset.
        assert end_y == pytest.approx(start["y"] - 120 + start["height"] * 0.75, abs=1)


def test_page_comment_reply_has_no_missing_target(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "index.html"
    _ = path.write_text("<h1>Review copy</h1>")
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        page.keyboard.press("/")
        field = overlay(page, "textarea")
        field.fill("A comment on the whole page")
        field.press("Enter")
        expect(overlay(page, "#badge")).to_have_text("1 queued")
        [record] = recv_all(tmp_path)
        assert record["selector"] == ""
        _ = jamparty(
            tmp_path, "send", "--anchor", str(record["id"]), "A page-wide reply."
        )
        answer = overlay(page, ".agent-message")
        expect(answer.locator(".agent-body")).to_have_text("A page-wide reply.")
        expect(answer.locator(".agent-unavailable")).to_be_hidden()
        expect(answer.get_by_role("link", name="Show")).to_have_count(0)
        answer.locator(".agent-body").click()
        expect(overlay(page, "#toast")).not_to_have_text("Element is unavailable")
        drag(page, answer.locator(".agent-header"), -100, -100)
        expect(answer).to_have_class("agent-message manually-placed")
        expect(answer.locator(".agent-unavailable")).to_be_hidden()


def test_answer_keyboard_move_and_reveal(tmp_path: Path, page: Page) -> None:
    without_answer_timeout(tmp_path)
    path = tmp_path / "index.html"
    _ = path.write_text(
        '<main style="padding: 240px 40px 900px">'
        '<h1 id="intro" style="width: 350px">Introduction</h1></main>'
    )
    with (
        share(tmp_path, "serve", str(path)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path,
            "send",
            "--at",
            "#intro",
            "--page",
            "/index.html",
            "See the intro.",
        )
        answer = overlay(page, "#agent-anchored .agent-message")
        expect(answer).to_have_attribute("tabindex", "0")
        expect(answer).to_be_visible()
        link = overlay(page, ".agent-anchor-line")
        original = link.get_attribute("d")
        answer.focus()
        answer.press("Tab")
        move_with_keys(answer, answer)
        expect(link).not_to_have_attribute("d", original or "")
        target = page.locator("#intro")
        moved = answer.bounding_box()
        assert moved is not None
        _ = jamparty(tmp_path, "send", "General note.")
        general = overlay(page, "#agent-general .agent-message")
        expect(general).to_have_text(re.compile("General note"))
        fixed = general.bounding_box()
        assert fixed is not None
        _ = page.evaluate("window.scrollTo(0, 160)")
        expect_position(answer, moved["x"], moved["y"] - 160)
        assert general.bounding_box() == fixed
        _ = page.evaluate("window.scrollTo(0, 650)")
        expect(target).not_to_be_in_viewport()
        expect(answer).not_to_be_in_viewport()
        _ = page.evaluate("window.scrollTo(0, 160)")
        expect(answer).to_be_visible()
        expect_position(answer, moved["x"], moved["y"] - 160)
        highlight = overlay(page, "#message-highlight")
        for key in ("Enter", "Space"):
            expect(highlight).to_be_hidden()
            _ = page.evaluate("window.scrollTo(0, 160)")
            expect(answer).to_be_visible()
            answer.focus()
            expect(answer).to_be_focused()
            page.keyboard.press(key)
            expect(highlight).to_be_visible()
            expect(target).to_be_in_viewport()
