"""Theme, drag and connector geometry in both overlay entry points."""

from pathlib import Path
from typing import cast

import pytest
from playwright.sync_api import Locator, Page, expect
from support import jamparty, join, overlay, recv_all, share
from test_answers import without_answer_timeout
from test_pdf import fixture_pdf


def drag(page: Page, handle: Locator, dx: int, dy: int) -> None:
    box = handle.bounding_box()
    assert box is not None
    x = box["x"] + box["width"] / 3
    y = box["y"] + box["height"] / 2
    page.mouse.move(x, y)
    page.mouse.down()
    page.mouse.move(x + dx, y + dy, steps=8)
    page.mouse.up()


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
        for selector in surfaces:
            node = overlay(page, selector)
            expect(node).to_have_css("background-color", "rgb(22, 22, 22)")
            assert node.evaluate(
                "(node) => getComputedStyle(node).fontFamily"
            ).startswith("system-ui")
        assert host.evaluate("(node) => getComputedStyle(node).fontFamily").startswith(
            "system-ui"
        )
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
        expect(answer).to_have_css("background-color", "rgb(255, 255, 255)")
        gear.click()
        overlay(page, "button[data-theme=dark]").click()
        expect(answer).to_have_css("background-color", "rgb(22, 22, 22)")
        gear.click()

        page.keyboard.press("/")
        field = overlay(page, "textarea")
        field.fill("A draft whose position I can change.")
        expect(field).to_be_focused()
        assert field.evaluate("(node) => getComputedStyle(node).fontFamily").startswith(
            "system-ui"
        )
        expect(field).to_have_css("box-shadow", "none")
        expect(field).to_have_css("outline-style", "solid")
        expect(field).to_have_css("outline-width", "2px")
        expect(field).to_have_css("border-top-color", "rgba(0, 0, 0, 0)")
        dialog = overlay(page, "#jamparty-dialog")
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
                    "(p) => { const b = p.getBBox(); return [b.x, b.y, b.width, b.height]; }"
                ),
            )
            assert brace_box[1] == pytest.approx(bounds["y"], abs=0.1)
            assert brace_box[3] == pytest.approx(bounds["height"], abs=0.1)
            assert brace_box[0] + brace_box[2] < bounds["x"]
            endpoints = cast(
                list[float],
                link.evaluate("""p => {
                    const start = p.getPointAtLength(0);
                    const end = p.getPointAtLength(p.getTotalLength());
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
        drag(page, heading, 0, -130)
        expect(link).not_to_have_attribute("d", original or "")
        moved = answer.bounding_box()
        assert moved is not None
        assert moved["y"] == pytest.approx(start["y"] - 130, abs=1)
        check_link()
        manual = link.get_attribute("d")
        _ = page.evaluate("window.scrollBy(0, 75)")
        expect(link).not_to_have_attribute("d", manual or "")
        assert answer.bounding_box() == moved
        check_link()


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
