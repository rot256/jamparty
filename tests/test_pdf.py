"""The joiner's PDF viewer, anchors and live rebuilds."""

import re
from base64 import b64decode
from collections.abc import Sequence
from pathlib import Path
from typing import Literal

import pytest
from playwright.sync_api import Page, Route, expect
from support import SocketTap, jamparty, join, overlay, recv_all, share, waiting


def fixture_pdf(
    label: str,
    padding: int = 0,
    *,
    outline_child: str | None = None,
    title: str | None = None,
) -> bytes:
    """Two letter-size pages with PDF-standard Helvetica and a valid xref."""
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        *(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            f"/Resources << /Font << /F1 5 0 R >> >> /Contents {stream} 0 R >>".encode()
            for stream in (6, 7)
        ),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    for text in ("First page", label):
        stream = f"BT /F1 24 Tf 72 648 Td ({text}) Tj ET".encode()
        objects.append(
            f"<< /Length {len(stream)} >>\nstream\n".encode() + stream + b"\nendstream"
        )
    if outline_child is not None:
        objects[0] = (
            b"<< /Type /Catalog /Pages 2 0 R /Outlines 8 0 R "
            b"/Dests << /appendix [4 0 R /XYZ null 560 null] >> >>"
        )
        objects.extend(
            [
                b"<< /Type /Outlines /First 9 0 R /Last 10 0 R /Count 3 >>",
                b"<< /Title (Introduction) /Parent 8 0 R /Next 10 0 R /Dest [3 0 R /Fit] >>",
                (
                    b"<< /Title (Analysis) /Parent 8 0 R /Prev 9 0 R /First 11 0 R "
                    b"/Last 11 0 R /Count 1 /Dest [4 0 R /Fit] >>"
                ),
                f"<< /Title ({outline_child}) /Parent 10 0 R /Dest /appendix >>".encode(),
            ]
        )
    return make_pdf(objects, padding, title=title)


def pdf_stream(fields: bytes, data: bytes) -> bytes:
    return (
        b"<< "
        + fields
        + f" /Length {len(data)} >>\nstream\n".encode()
        + data
        + b"\nendstream"
    )


def make_pdf(
    objects: list[bytes], padding: int = 0, *, title: str | None = None
) -> bytes:
    info = ""
    if title is not None:
        title_hex = (b"\xfe\xff" + title.encode("utf-16-be")).hex()
        objects = [*objects, f"<< /Title <{title_hex}> >>".encode()]
        info = f" /Info {len(objects)} 0 R"
    pdf = bytearray(b"%PDF-1.4\n")
    if padding:
        pdf.extend((b"%" + b" " * 998 + b"\n") * (padding // 1000))
    offsets = [0]
    for number, obj in enumerate(objects, start=1):
        offsets.append(len(pdf))
        pdf.extend(f"{number} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        pdf.extend(f"{offset:010} 00000 n \n".encode())
    pdf.extend(
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R{info} >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(pdf)


def scroll_to_page(page: Page, number: int) -> None:
    sheet = page.locator(f"#jamparty-pdf-page-{number}")
    _ = sheet.evaluate("node => node.scrollIntoView()")
    expect(sheet).to_have_attribute("data-rendered", "true")


def expect_selection_cleared(page: Page) -> None:
    expect(
        page.locator(".pdf-draft, .pdf-drag, .pdf-block-hover, .pdf-comment")
    ).to_have_count(0)
    assert page.evaluate("window.getSelection().rangeCount") == 0


def send_comment(page: Page, message: str) -> None:
    field = overlay(page, "textarea")
    expect(field).to_be_focused()
    expect(page.locator(".pdf-draft")).to_have_count(1)
    expect(overlay(page, "#highlight")).to_be_hidden()
    field.fill(message)
    expect(page.locator(".pdf-draft")).to_have_count(1)
    field.press("Enter")
    expect(overlay(page, "#jamparty-dialog")).not_to_be_visible()
    expect_selection_cleared(page)
    expect(overlay(page, "#outbox-list li").first).to_have_attribute(
        "data-state", "queued"
    )


def test_pdf_fits_and_keeps_reading_position(tmp_path: Path, page: Page) -> None:
    pdf = tmp_path / "paper.pdf"
    _ = pdf.write_bytes(fixture_pdf("Second page"))
    page.emulate_media(reduced_motion="reduce")
    with (
        share(tmp_path, "serve", str(pdf)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        scroll_to_page(page, 2)
        sheet = page.get_by_role("region", name="Page 2")
        indicator = page.locator("#pdf-position")
        _ = page.evaluate("window.scrollBy(0, 200)")
        expect(indicator).to_have_text("2 / 2")
        expect(indicator).to_be_visible()

        page.set_viewport_size({"width": 360, "height": 720})
        expect(sheet).to_have_css("width", "328px")
        expect(sheet).to_have_attribute("data-rendered", "true")
        bounds = sheet.bounding_box()
        assert bounds is not None
        assert bounds["width"] == pytest.approx(328, abs=1)
        assert (12 - bounds["y"]) / bounds["height"] == pytest.approx(
            200 / (960 * 792 / 612), abs=0.004
        )
        assert page.evaluate(
            "document.documentElement.scrollWidth === document.documentElement.clientWidth"
        )
        expect(indicator).to_be_hidden()


def test_pdf_tab_title_tracks_rebuilds(tmp_path: Path, page: Page) -> None:
    pdf = tmp_path / "paper #%.pdf"
    _ = pdf.write_bytes(fixture_pdf("First version", title="Research notes"))
    with (
        share(tmp_path, "serve", str(pdf)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(page).to_have_title("Research notes")
        updated = tmp_path / "paper.tmp"
        _ = updated.write_bytes(fixture_pdf("Second version", title="Revised notes"))
        _ = updated.replace(pdf)
        expect(page).to_have_title("Revised notes")
        _ = updated.write_bytes(fixture_pdf("Third version"))
        _ = updated.replace(pdf)
        expect(page).to_have_title("paper #%.pdf")


def test_pdf_outline(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "build"
    site.mkdir()
    pdf = site / "paper.pdf"
    _ = pdf.write_bytes(fixture_pdf("Second page", outline_child="<b>Appendix</b>"))
    _ = (site / "index.html").write_text("<h1>Start page</h1>")
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#edit")).to_be_visible()
        expect(overlay(page, "#pdf-outline")).to_have_count(0)
        _ = page.goto(f"{joined.url.rstrip('/')}/paper.pdf")
        sections = overlay(page, "#pdf-outline")
        panel = overlay(page, "#pdf-sections")
        expect(sections).to_be_visible()
        expect(panel).to_be_hidden()
        sections.click()

        intro = panel.get_by_role("button", name="Introduction")
        analysis = panel.get_by_role("button", name="Analysis", exact=True)
        appendix = panel.get_by_role("button", name="<b>Appendix</b>")
        disclosure = panel.get_by_role("button", name="Subsections of Analysis")
        expect(sections).to_have_attribute("aria-expanded", "true")
        expect(intro).to_have_attribute("aria-current", "location")
        expect(appendix).to_be_hidden()
        analysis.click()
        expect(panel).to_be_hidden()
        expect(page.locator("#pdf-position")).to_have_text("2 / 2")

        sections.click()
        expect(analysis).to_have_attribute("aria-current", "location")
        disclosure.click()
        appendix.click()
        expect(panel).to_be_hidden()
        sheet = page.get_by_role("region", name="Page 2")
        expect(sheet).to_have_attribute("data-rendered", "true")
        bounds = sheet.bounding_box()
        assert bounds is not None
        assert (12 - bounds["y"]) / bounds["height"] == pytest.approx(
            232 / 792, abs=0.002
        )
        sections.click()
        expect(appendix).to_have_attribute("aria-current", "location")

        updated = site / "paper.tmp"
        _ = updated.write_bytes(
            fixture_pdf("Second page", outline_child="Revised appendix")
        )
        _ = updated.replace(pdf)
        expect(panel.get_by_role("button", name="Revised appendix")).to_be_visible()
        expect(disclosure).to_have_attribute("aria-expanded", "true")
        expect(panel).to_be_visible()
        page.keyboard.press("Escape")
        expect(panel).to_be_hidden()
        expect(sections).to_be_focused()

        sections.click()
        overlay(page, "#gear").click()
        expect(panel).to_be_hidden()
        expect(overlay(page, "#panel")).to_be_visible()
        sections.click()
        expect(overlay(page, "#panel")).to_be_hidden()
        page.mouse.click(10, 100)
        expect(panel).to_be_hidden()

        sections.click()
        _ = updated.write_bytes(fixture_pdf("No outline"))
        _ = updated.replace(pdf)
        expect(sections).to_be_hidden()
        expect(panel).to_be_hidden()
        _ = updated.write_bytes(fixture_pdf("Outline again", outline_child="Appendix"))
        _ = updated.replace(pdf)
        expect(sections).to_be_visible()
        expect(panel).to_be_hidden()


def test_pdf_without_outline(tmp_path: Path, page: Page) -> None:
    pdf = tmp_path / "paper.pdf"
    _ = pdf.write_bytes(fixture_pdf("Second page"))
    with (
        share(tmp_path, "serve", str(pdf)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(page.locator("#jamparty-pdf-page-1")).to_have_attribute(
            "data-rendered", "true"
        )
        expect(overlay(page, "#pdf-outline")).to_be_hidden()
        expect(overlay(page, "#pdf-sections")).to_be_hidden()


def test_pdf_comments_and_rebuild(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "build"
    site.mkdir()
    pdf = site / "paper.PDF"
    _ = pdf.write_bytes(fixture_pdf("Second page"))
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.set_viewport_size({"width": 848, "height": 720})
        url = f"{joined.url.rstrip('/')}/paper.PDF?review=1"
        response = page.goto(url)
        assert response is not None
        assert "default-src 'none'" in response.headers["content-security-policy"]
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(page.locator("#jamparty-pdf-page-1")).to_have_attribute(
            "data-rendered", "true"
        )

        scroll_to_page(page, 2)
        sheet = page.locator("#jamparty-pdf-page-2")
        expect(sheet).to_have_attribute("data-rendered", "true")
        expect(sheet.locator(".textLayer span")).to_have_text("Second page")
        # At 100%, 4/3 CSS pixels correspond to one PDF point.
        box = sheet.bounding_box()
        assert box is not None
        x, y = box["x"], box["y"]
        overlay(page, "#edit").click()
        page.mouse.move(x + 110, y + 180)
        expect(sheet.locator(".pdf-block-hover")).to_be_visible()
        page.mouse.move(x + 96, y + 256)
        page.mouse.down()
        page.mouse.move(x + 288, y + 384, steps=5)
        page.mouse.up()
        send_comment(page, "Move this figure.")
        overlay(page, "#edit").click()
        expect_selection_cleared(page)
        overlay(page, "#pill").click()
        row = overlay(page, "#outbox-list li")
        row.hover()
        expect(sheet.locator(".pdf-comment")).to_have_count(1)
        [box_record] = recv_all(tmp_path)
        expect(row).to_have_attribute("data-state", "delivered")
        expect(sheet.locator(".pdf-comment")).to_have_count(1)
        overlay(page, "#collapse").hover()
        expect_selection_cleared(page)
        row.focus()
        expect(sheet.locator(".pdf-comment")).to_have_count(1)
        overlay(page, "#collapse").click()
        expect_selection_cleared(page)
        assert box_record["page"] == url
        assert box_record["pdf"] == {
            "page": 2,
            "box": {"x0": 72, "y0": 192, "x1": 216, "y1": 288},
        }
        assert "selectedText" not in box_record

        text = sheet.locator(".textLayer span")
        _ = text.evaluate(
            """node => {
              const range = document.createRange();
              range.selectNodeContents(node);
              window.getSelection().removeAllRanges();
              window.getSelection().addRange(range);
            }"""
        )
        page.keyboard.press("/")
        send_comment(page, "Rename the heading.")
        [text_record] = recv_all(tmp_path)
        assert text_record["selectedText"] == "Second page"
        anchor = text_record["pdf"]
        assert isinstance(anchor, dict)
        assert anchor["page"] == 2
        bounds = anchor["box"]
        assert isinstance(bounds, dict)
        assert bounds["x0"] == pytest.approx(72, abs=1)
        assert 120 < bounds["y0"] < 144
        assert bounds["x1"] > 180
        assert 143 < bounds["y1"] < 156
        expect(sheet.locator(".pdf-comment")).to_have_count(0)
        _ = jamparty(tmp_path, "send", "--anchor", str(box_record["id"]), "Moved.")
        expect(overlay(page, ".agent-message.has-anchor")).to_contain_text("Moved.")

        _ = page.evaluate("window.scrollBy(0, 90)")
        scroll = float(page.evaluate("window.scrollY"))
        updated = site / "paper.tmp"
        _ = updated.write_bytes(fixture_pdf("Rebuilt page"))
        _ = updated.replace(pdf)
        expect(sheet.locator(".textLayer span")).to_have_text("Rebuilt page")
        assert float(page.evaluate("window.scrollY")) == pytest.approx(scroll, abs=2)
        expect(sheet.locator(".pdf-comment")).to_have_count(0)
        rebuilt = sheet.bounding_box()
        assert rebuilt is not None
        overlay(page, "#edit").click()
        page.mouse.move(rebuilt["x"] + 110, rebuilt["y"] + 180)
        expect(sheet.locator(".pdf-block-hover")).to_be_visible()
        page.mouse.click(rebuilt["x"] + 110, rebuilt["y"] + 180)
        send_comment(page, "Review the rebuilt block.")
        [block_record] = recv_all(tmp_path)
        assert block_record["selectedText"] == "Rebuilt page"

        overlay(page, "#edit").click()
        # Keep the draft marker through a document swap, resize and scroll.
        _ = sheet.locator(".textLayer span").evaluate("""node => {
          const range = document.createRange();
          range.selectNodeContents(node);
          window.getSelection().addRange(range);
        }""")
        page.keyboard.press("/")
        field = overlay(page, "textarea")
        field.fill("Keep this draft")
        expect(sheet.locator(".pdf-draft")).to_have_count(1)
        _ = updated.write_bytes(
            reading_pdf(
                [(792, 144, ["First page"]), (792, 144, ["Third version"])]
                + [(792, 144, ["Another page"])] * 4
            )
        )
        _ = updated.replace(pdf)
        expect(sheet.locator(".textLayer span")).to_have_text("Third version")
        expect(page.locator(".pdf-draft")).to_have_count(1)
        page.set_viewport_size({"width": 644, "height": 720})
        _ = page.evaluate("window.scrollBy(0, 75)")
        expect(sheet.locator(".pdf-draft")).to_have_count(1)
        expect(field).to_have_value("Keep this draft")
        field.press("Escape")
        expect_selection_cleared(page)
        for number in (6, 2):
            scroll_to_page(page, number)
            if number == 6:
                expect(sheet).not_to_have_attribute("data-rendered", "true")
            expect(
                page.locator(".pdf-comment, .pdf-draft, .pdf-drag, .pdf-block-hover")
            ).to_have_count(0)
        expect(page.locator(".pdf-comment-anchor")).to_have_count(3)
        overlay(page, "#pill").click()
        rows = overlay(page, "#outbox-list li")
        rows.nth(0).hover()
        expect(page.locator(".pdf-comment")).to_have_count(1)
        hovered = page.locator(".pdf-comment").get_attribute("id")
        # Focus moving to another row wins over the resting pointer.
        rows.nth(1).focus()
        expect(page.locator(".pdf-comment")).to_have_count(1)
        assert page.locator(".pdf-comment").get_attribute("id") != hovered
        overlay(page, "#collapse").hover()
        overlay(page, "#collapse").focus()
        expect(page.locator(".pdf-comment")).to_have_count(0)


@pytest.mark.parametrize("tool", ["slash", "contextmenu", "block", "box"])
@pytest.mark.parametrize("cancelled", [False, True])
def test_pdf_dialog_clears_draft(
    tmp_path: Path, page: Page, tool: str, cancelled: bool
) -> None:
    pdf = tmp_path / "paper.pdf"
    _ = pdf.write_bytes(fixture_pdf("Second page"))
    with (
        share(tmp_path, "serve", str(pdf)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        sheet = page.locator("#jamparty-pdf-page-1")
        text = sheet.locator(".textLayer span")
        expect(text).to_have_text("First page")
        box = text.bounding_box()
        assert box is not None
        x, y = box["x"] + 5, box["y"] + box["height"] / 2
        if tool in ("slash", "contextmenu"):
            page.mouse.move(x, y)
            page.mouse.down()
            page.mouse.move(x + box["width"] - 10, y, steps=10)
            page.mouse.up()
            assert page.evaluate("window.getSelection().toString().length") > 0
            if tool == "slash":
                page.keyboard.press("/")
            else:
                page.mouse.click(x + 10, y, button="right")
        else:
            overlay(page, "#edit").click()
            page.mouse.move(x, y)
            expect(sheet.locator(".pdf-block-hover")).to_be_visible()
            if tool == "block":
                page.mouse.click(x, y)
            else:
                page.mouse.down()
                page.mouse.move(x + 40, y + 20, steps=5)
                page.mouse.up()

        field = overlay(page, "textarea")
        expect(field).to_be_focused()
        field.fill("Draft")
        expect(sheet.locator(".pdf-draft")).to_be_visible()
        expect(page.locator(".pdf-draft")).to_have_count(1)
        expect(overlay(page, "#highlight")).to_be_hidden()
        field.press("Escape" if cancelled else "Enter")
        expect(field).to_be_hidden()
        expect_selection_cleared(page)


@pytest.mark.parametrize("button", ["left", "right"])
def test_pdf_click_comments(
    tmp_path: Path, page: Page, button: Literal["left", "right"]
) -> None:
    pdf = tmp_path / "paper.pdf"
    _ = pdf.write_bytes(fixture_pdf("Second page"))
    with (
        share(tmp_path, "serve", str(pdf)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.set_viewport_size({"width": 848, "height": 720})
        _ = page.goto(joined.url)
        sheet = page.locator("#jamparty-pdf-page-1")
        text = sheet.locator(".textLayer span")
        expect(text).to_have_text("First page")
        expect(sheet).to_have_attribute("data-rendered", "true")
        if button == "left":
            overlay(page, "#edit").click()

        bounds = sheet.bounding_box()
        assert bounds is not None
        x, y = bounds["x"], bounds["y"]
        # At 100% zoom this is (4.5, 192) PDF points from the upper left.
        if button == "left":
            page.mouse.move(x + 5, y + 256)
            page.mouse.down()
            page.mouse.move(x + 6, y + 256)
            page.mouse.up()
        else:
            page.mouse.click(x + 6, y + 256, button=button)
        send_comment(page, "Review this point.")
        [point] = recv_all(tmp_path)
        assert point["pdf"] == {
            "page": 1,
            "box": {"x0": 0, "y0": 184, "x1": 12.5, "y1": 200},
        }
        assert "selectedText" not in point

        block = text.bounding_box()
        assert block is not None
        page.mouse.click(
            block["x"] + 5, block["y"] + block["height"] / 2, button=button
        )
        send_comment(page, "Review this block.")
        [record] = recv_all(tmp_path)
        assert record["selectedText"] == "First page"
        anchor = record["pdf"]
        assert isinstance(anchor, dict)
        assert anchor["page"] == 1
        block_bounds = anchor["box"]
        assert isinstance(block_bounds, dict)
        assert block_bounds["x0"] == 72
        assert block_bounds["x1"] > 130

        if button == "left":
            # Place the gap between pages in the viewport.
            _ = sheet.evaluate(
                "node => window.scrollBy(0, node.getBoundingClientRect().bottom - 300)"
            )
            bounds = sheet.bounding_box()
            assert bounds is not None
            page.mouse.click(bounds["x"] + 100, bounds["y"] + bounds["height"] + 6)
            expect(overlay(page, "#jamparty-dialog")).to_be_hidden()
            expect(page.locator(".pdf-draft")).to_have_count(0)


@pytest.mark.parametrize("button", ["left", "right"])
@pytest.mark.parametrize("target", ["block", "point"])
def test_pdf_click_comments_while_scrolling(
    tmp_path: Path, page: Page, button: Literal["left", "right"], target: str
) -> None:
    pdf = tmp_path / "paper.pdf"
    _ = pdf.write_bytes(fixture_pdf("Second page"))
    # Hold the next PDF.js text request after the initial render.
    page.add_init_script("""
        const post = Worker.prototype.postMessage;
        Worker.prototype.postMessage = function (message, ...args) {
            if (window.holdPdfText && message.action === "GetTextContent") {
                window.holdPdfText = false;
                window.releasePdfText = () => post.call(this, message, ...args);
                return;
            }
            return post.call(this, message, ...args);
        };
    """)
    with (
        share(tmp_path, "serve", str(pdf)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.set_viewport_size({"width": 848, "height": 720})
        _ = page.goto(joined.url)
        sheet = page.locator("#jamparty-pdf-page-1")
        expect(sheet).to_have_attribute("data-rendered", "true")
        if button == "left":
            overlay(page, "#edit").click()
        _ = page.evaluate("window.holdPdfText = true")

        if target == "block":
            bounds = sheet.locator(".textLayer span").bounding_box()
            assert bounds is not None
            x = bounds["x"] + 5
            y = bounds["y"] + bounds["height"] / 2
        else:
            bounds = sheet.bounding_box()
            assert bounds is not None
            x, y = bounds["x"] + 6, bounds["y"] + 256
        page.mouse.click(x, y, button=button)
        _ = page.wait_for_function("typeof window.releasePdfText === 'function'")
        expect(overlay(page, "#jamparty-dialog")).to_be_hidden()
        _ = page.evaluate("""
            () => new Promise(resolve => {
                window.addEventListener("scroll", () => resolve(window.scrollY), { once: true });
                window.scrollBy(0, 100);
            })
        """)
        _ = page.evaluate("window.releasePdfText()")

        send_comment(page, "Review the original location.")
        [record] = recv_all(tmp_path)
        if target == "block":
            assert record["selectedText"] == "First page"
            anchor = record["pdf"]
            assert isinstance(anchor, dict)
            assert anchor["page"] == 1
            box = anchor["box"]
            assert isinstance(box, dict)
            assert box["x0"] == 72
        else:
            assert "selectedText" not in record
            assert record["pdf"] == {
                "page": 1,
                "box": {"x0": 0, "y0": 184, "x1": 12.5, "y1": 200},
            }


def test_rotated_pdf_anchors(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "build"
    site.mkdir()
    _ = (site / "rotated.pdf").write_bytes(
        make_pdf(
            [
                b"<< /Type /Catalog /Pages 2 0 R >>",
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                (
                    b"<< /Type /Page /Parent 2 0 R /MediaBox [10 20 410 620] "
                    b"/CropBox [35 70 385 560] /Rotate 90 "
                    b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
                ),
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
                pdf_stream(b"", b"BT /F1 24 Tf 72 480 Td (ROTATION) Tj ET"),
            ]
        )
    )
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.set_viewport_size({"width": 685, "height": 720})
        _ = page.goto(f"{joined.url.rstrip('/')}/rotated.pdf")
        sheet = page.locator("#jamparty-pdf-page-1")
        expect(sheet).to_have_attribute("data-rendered", "true")
        box = sheet.bounding_box()
        assert box is not None
        x, y = box["x"], box["y"]
        assert box["width"] == pytest.approx(490 * 4 / 3, abs=1)
        assert box["height"] == pytest.approx(350 * 4 / 3, abs=1)

        # The visible baseline is at x=(480-70)*4/3, with the ink to its
        # right. Drag down across the glyphs using PDF coordinates only.
        page.mouse.move(x + 557, y + 52)
        page.mouse.down()
        page.mouse.move(x + 557, y + 213, steps=15)
        page.mouse.up()
        page.keyboard.press("/")
        send_comment(page, "Rename this heading.")
        [text_record] = recv_all(tmp_path)
        assert text_record["selectedText"] == "ROTATION"
        assert text_record["pdf"] == {
            "page": 1,
            "box": {
                "x0": pytest.approx(62, abs=1),
                "x1": pytest.approx(186, abs=2),
                "y0": pytest.approx(119, abs=2),
                "y1": pytest.approx(148, abs=3),
            },
        }

        overlay(page, "#edit").click()
        page.mouse.move(x + 557, y + 90)
        expect(sheet.locator(".pdf-block-hover")).to_be_visible()
        page.mouse.click(x + 557, y + 90)
        send_comment(page, "Select the rotated heading.")
        [block_record] = recv_all(tmp_path)
        assert block_record["selectedText"] == "ROTATION"
        assert block_record["pdf"] == {
            "page": 1,
            "box": {
                "x0": pytest.approx(62, abs=1),
                "x1": pytest.approx(186, abs=2),
                "y0": pytest.approx(122, abs=2),
                "y1": pytest.approx(145, abs=2),
            },
        }
        page.mouse.move(x + 80, y + 40)
        page.mouse.down()
        page.mouse.move(x + 200, y + 160, steps=5)
        page.mouse.up()
        send_comment(page, "Move this box.")
        [box_record] = recv_all(tmp_path)
        assert box_record["pdf"] == {
            "page": 1,
            "box": {
                # Firefox rounds coordinates on pointer dispatch.
                "x0": pytest.approx(55, abs=0.5),
                "y0": pytest.approx(400, abs=0.5),
                "x1": pytest.approx(145, abs=0.5),
                "y1": pytest.approx(490, abs=0.5),
            },
        }


def test_pdf_jpeg2000_renders_visible_pixels(tmp_path: Path, page: Page) -> None:
    # An 8×8 RGB (220, 20, 60) image encoded as a raw JPEG 2000 codestream.
    image = b64decode(
        "/0//UQAvAAAAAAAIAAAACAAAAAAAAAAAAAAACAAAAAgAAAAAAAAAAAADBwEBBwEBBwEB"
        "/1IADAAAAAEAAwQEAAH/XAANQEBISFBISFBISFD/ZAAlAAFDcmVhdGVkIGJ5IE9wZW5K"
        "UEVHIHZlcnNpb24gMi41LjT/kAAKAAAAAAAmAAH/k8+0CAMzz7QICB/PtAgHL4CAgICAgICAgP/Z"
    )
    site = tmp_path / "build"
    site.mkdir()
    _ = (site / "image.pdf").write_bytes(
        make_pdf(
            [
                b"<< /Type /Catalog /Pages 2 0 R >>",
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                (
                    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
                    b"/Resources << /XObject << /Im 4 0 R >> >> /Contents 5 0 R >>"
                ),
                pdf_stream(
                    b"/Type /XObject /Subtype /Image /Width 8 /Height 8 /Filter /JPXDecode",
                    image,
                ),
                pdf_stream(b"", b"100 0 0 100 50 50 cm /Im Do"),
            ]
        )
    )
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(f"{joined.url.rstrip('/')}/image.pdf")
        sheet = page.locator("#jamparty-pdf-page-1")
        expect(sheet).to_have_attribute("data-rendered", "true")
        assert sheet.locator("canvas").evaluate("""canvas => {
          const {width, height} = canvas;
          return Array.from(canvas.getContext("2d").getImageData(width/2, height/2, 1, 1).data);
        }""") == [220, 20, 60, 255]


def test_pdf_named_cmap_renders_and_extracts_text(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "build"
    site.mkdir()
    _ = (site / "named.pdf").write_bytes(
        make_pdf(
            [
                b"<< /Type /Catalog /Pages 2 0 R >>",
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                (
                    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] "
                    b"/Resources << /Font << /F1 4 0 R >> >> /Contents 7 0 R >>"
                ),
                (
                    b"<< /Type /Font /Subtype /Type0 /BaseFont /HeiseiKakuGo-W5 "
                    b"/Encoding /UniJIS-UCS2-H /DescendantFonts [5 0 R] >>"
                ),
                (
                    b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /HeiseiKakuGo-W5 "
                    b"/CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 2 >> "
                    b"/FontDescriptor 6 0 R /DW 1000 >>"
                ),
                (
                    b"<< /Type /FontDescriptor /FontName /HeiseiKakuGo-W5 /Flags 4 "
                    b"/FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 880 /Descent -120 "
                    b"/CapHeight 700 /StemV 80 >>"
                ),
                pdf_stream(b"", b"BT /F1 36 Tf 40 100 Td <004100420043> Tj ET"),
            ]
        )
    )
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        url = joined.url.rstrip("/")
        with page.expect_response(
            f"{url}/_jamparty/pdf-cmaps/UniJIS-UCS2-H.bcmap"
        ) as cmap:
            _ = page.goto(f"{url}/named.pdf")
        assert cmap.value.ok
        sheet = page.locator("#jamparty-pdf-page-1")
        expect(sheet).to_have_attribute("data-rendered", "true")
        expect(sheet.locator(".textLayer")).to_have_text("ABC")
        assert sheet.locator("canvas").evaluate("""canvas => {
          const {width, height} = canvas;
          const pixels = canvas.getContext("2d").getImageData(0, 0, width, height).data;
          return pixels.some((value, index) => index % 4 === 0 && value < 100);
        }""")


def test_pdf_failed_refresh_retries_on_same_revision(
    tmp_path: Path, page: Page
) -> None:
    site = tmp_path / "build"
    site.mkdir()
    pdf = site / "paper.pdf"
    _ = pdf.write_bytes(fixture_pdf("Old page"))
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        tap = SocketTap(page)
        url = f"{joined.url.rstrip('/')}/paper.pdf"
        _ = page.goto(url)
        text = page.locator("#jamparty-pdf-page-2 .textLayer span")
        expect(text).to_have_text("Old page")
        revisions: list[object] = []

        def observe(message: dict[str, object]) -> dict[str, object]:
            if message["type"] in {"hello", "reload"}:
                revisions.append(message["revision"])
            return message

        tap.edit = observe

        def fail(route: Route) -> None:
            route.abort("connectionreset")

        page.route(url, fail)
        _ = pdf.write_bytes(fixture_pdf("New page"))
        expect(page.locator("#pdf-status")).to_have_text(
            "Could not open the updated PDF."
        )
        expect(text).to_have_text("Old page")
        page.unroute(url, fail)
        tap.sockets[-1].close()
        waiting(page, "the socket to reopen", lambda: len(tap.sockets) == 2)
        expect(text).to_have_text("New page")
        assert revisions == [1, 1]


@pytest.mark.parametrize("with_length", [True, False])
def test_pdf_byte_limit(tmp_path: Path, page: Page, with_length: bool) -> None:
    site = tmp_path / "build"
    site.mkdir()
    # Wrap the real fetch in the browser to simulate a large response without
    # allocating an oversized fixture or depending on proxy buffering.
    page.add_init_script(f"""(() => {{
      const fetchPdf = window.fetch;
      window.fetch = async (...args) => {{
        const response = await fetchPdf(...args);
        if (!response.url.endsWith("/large.pdf")) return response;
        await response.body.cancel();
        const chunk = new Uint8Array(1024 * 1024);
        let reads = 0;
        const body = new ReadableStream({{
          pull(controller) {{
            reads++;
            controller.enqueue(chunk);
            if (reads === 130) controller.close();
          }},
          cancel() {{ document.documentElement.dataset.pdfReads = String(reads); }},
        }}, {{highWaterMark: 0}});
        return new Response(body, {{headers: {{"test": "large",
          ...({str(with_length).lower()} ? {{"Content-Length": "135266304"}} : {{}})
        }}}});
      }};
    }})();""")
    _ = (site / "large.pdf").write_bytes(fixture_pdf("Unused"))
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(f"{joined.url.rstrip('/')}/large.pdf")
        expect(page.locator("#pdf-status")).to_have_text(
            "This PDF exceeds the 128 MiB viewer limit."
        )
        expect(page.locator("html")).to_have_attribute(
            "data-pdf-reads", "0" if with_length else "129"
        )
        expect(page.locator(".pdf-page")).to_have_count(0)


def blank_pdf(page_count: int) -> bytes:
    children = " ".join(f"{i} 0 R" for i in range(3, 3 + page_count))
    return make_pdf(
        [
            b"<< /Type /Catalog /Pages 2 0 R >>",
            f"<< /Type /Pages /Kids [{children}] /Count {page_count} >>".encode(),
            *(
                [b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>"]
                * page_count
            ),
        ]
    )


def test_pdf_pages_load_on_demand(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "build"
    site.mkdir()
    _ = (site / "long.pdf").write_bytes(blank_pdf(100))
    page.add_init_script("""(() => {
      const send = Worker.prototype.postMessage;
      Worker.prototype.postMessage = function (message, ...args) {
        if (message.action === "GetPage") {
          const root = document.documentElement;
          root.dataset.pdfPages = (root.dataset.pdfPages ?? "") + message.data.pageIndex + ",";
        }
        return send.call(this, message, ...args);
      };
    })();""")
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(f"{joined.url.rstrip('/')}/long.pdf")
        expect(page.locator("#jamparty-pdf-page-1")).to_have_attribute(
            "data-rendered", "true"
        )
        expect(page.locator(".pdf-page")).to_have_count(100)
        requests = page.locator("html").get_attribute("data-pdf-pages")
        assert requests is not None and "89" not in requests.split(",")
        scroll_to_page(page, 90)
        expect(page.locator("#pdf-position")).to_have_text("90 / 100")
        requests = page.locator("html").get_attribute("data-pdf-pages")
        assert requests is not None and "89" in requests.split(",")

        _ = page.evaluate("delete document.documentElement.dataset.pdfPages")
        updated = site / "long.tmp"
        _ = updated.write_bytes(blank_pdf(101))
        _ = updated.replace(site / "long.pdf")
        expect(page.locator(".pdf-page")).to_have_count(101)
        expect(page.locator("#jamparty-pdf-page-90")).to_have_attribute(
            "data-rendered", "true"
        )
        expect(page.locator("#pdf-position")).to_have_text("90 / 101")
        requests = page.locator("html").get_attribute("data-pdf-pages")
        assert requests is not None
        assert "89" in requests.split(",") and "49" not in requests.split(",")

        _ = updated.write_bytes(blank_pdf(1001))
        _ = updated.replace(site / "long.pdf")
        expect(page.locator("#pdf-status")).to_have_text(
            "This PDF exceeds the 1000-page viewer limit."
        )
        expect(page.locator(".pdf-page")).to_have_count(101)


def test_pdf_evicts_rendered_images(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "build"
    site.mkdir()
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R 10 0 R 12 0 R 14 0 R] /Count 6 >>",
        pdf_stream(b"", b"q 600 0 0 792 0 0 cm /Im Do Q"),
    ]
    for image in range(5, 16, 2):
        objects.extend(
            [
                (
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 792] "
                    f"/Resources << /XObject << /Im {image} 0 R >> >> /Contents 3 0 R >>"
                ).encode(),
                pdf_stream(
                    b"/Type /XObject /Subtype /Image /Width 256 /Height 256 "
                    b"/ColorSpace /DeviceRGB /BitsPerComponent 8",
                    bytes((205, 80, 44)) * (256 * 256),
                ),
            ]
        )
    _ = (site / "images.pdf").write_bytes(make_pdf(objects))
    page.add_init_script("""(() => {
      const send = Worker.prototype.postMessage;
      const rendered = new Map();
      Worker.prototype.postMessage = function (message, ...args) {
        const root = document.documentElement;
        if (message.action === "GetOperatorList") {
          const { pageIndex, pageProxyId } = message.data;
          rendered.set(pageProxyId, pageIndex + 1);
          root.dataset.pdfRenders = (root.dataset.pdfRenders ?? "") + pageIndex + ",";
        }
        if (message.action === "CleanupPage") {
          const number = rendered.get(message.data.pageProxyId);
          root.dataset.pdfEvicted = (root.dataset.pdfEvicted ?? "") + number + ",";
        }
        return send.call(this, message, ...args);
      };
    })();""")
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(f"{joined.url.rstrip('/')}/images.pdf")
        root = page.locator("html")
        for number in (1, 6, 1):
            scroll_to_page(page, number)
            sheet = page.locator(f"#jamparty-pdf-page-{number}")
            assert sheet.locator("canvas").evaluate(
                """canvas => Array.from(canvas.getContext('2d')
                    .getImageData(canvas.width / 2, canvas.height / 2, 1, 1).data)"""
            ) == [205, 80, 44, 255]
        evicted = (root.get_attribute("data-pdf-evicted") or "").split(",")
        assert {"1", "6"} <= set(evicted)
        renders = (root.get_attribute("data-pdf-renders") or "").split(",")
        assert renders.count("0") >= 2


def blocks_pdf(tagged: bool) -> bytes:
    catalog = b"<< /Type /Catalog /Pages 2 0 R"
    sheet = (
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 450] "
        b"/Resources << /Font << /F1 4 0 R >> /XObject << /Im 6 0 R >> >> "
        b"/Contents 5 0 R"
    )
    if tagged:
        catalog += b" /StructTreeRoot 7 0 R /MarkInfo << /Marked true >>"
        sheet += b" /StructParents 0"
    lines = [
        (22, 60, 402, "Review notes"),
        (12, 60, 360, "First paragraph starts here."),
        (12, 60, 346, "This is the second line."),
        (12, 340, 360, "Right column starts here."),
        (12, 340, 346, "It has a second line too."),
        (12, 60, 310, "A separate paragraph."),
        (12, 265, 260, "F = m times a"),
    ]
    commands: list[bytes] = []
    for index, (size, x, y, label) in enumerate(lines):
        if tagged:
            commands.append(f"/Span <</MCID {index}>> BDC".encode())
        commands.append(f"BT /F1 {size} Tf {x} {y} Td ({label}) Tj ET".encode())
        if tagged:
            commands.append(b"EMC")
    if tagged:
        commands.append(b"/Figure <</MCID 7>> BDC")
    commands.append(b"q 75 0 0 44 85 130 cm /Im Do Q")
    if tagged:
        commands.append(b"EMC")
        commands.append(
            b"/Span <</MCID 8>> BDC BT /F1 10 Tf 85 115 Td (Sample image) Tj ET EMC"
        )
    objects = [
        catalog + b" >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        sheet + b" >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        pdf_stream(b"", b"\n".join(commands)),
        pdf_stream(
            b"/Type /XObject /Subtype /Image /Width 2 /Height 2 "
            b"/ColorSpace /DeviceRGB /BitsPerComponent 8",
            bytes((205, 80, 44)) * 4,
        ),
    ]
    if tagged:
        # Group the widely separated left-column lines as a single tagged P.
        elements = [
            ("H1", "0"),
            ("P", "[1 2 5]"),
            ("P", "[3 4]"),
            ("Formula", "6"),
            ("Figure", "[7 8]"),
        ]
        objects.extend(
            [
                (
                    b"<< /Type /StructTreeRoot /K [9 0 R 10 0 R 11 0 R 12 0 R 13 0 R] "
                    b"/ParentTree 8 0 R >>"
                ),
                (
                    b"<< /Nums [0 [9 0 R 10 0 R 10 0 R 11 0 R 11 0 R 10 0 R "
                    b"12 0 R 13 0 R 13 0 R]] >>"
                ),
                *(
                    f"<< /Type /StructElem /P 7 0 R /Pg 3 0 R /S /{role} /K {kids} >>".encode()
                    for role, kids in elements
                ),
            ]
        )
    return make_pdf(objects)


@pytest.mark.parametrize("tagged", [False, True])
def test_pdf_block_selection(tmp_path: Path, page: Page, tagged: bool) -> None:
    site = tmp_path / "build"
    site.mkdir()
    _ = (site / "blocks.pdf").write_bytes(blocks_pdf(tagged))
    page.add_init_script("""(() => {
      const send = Worker.prototype.postMessage;
      Worker.prototype.postMessage = function (message, ...args) {
        if (message.action === "GetStructTree") {
          document.documentElement.dataset.pdfRequestedTree = "yes";
        }
        return send.call(this, message, ...args);
      };
    })();""")
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.set_viewport_size({"width": 632, "height": 720})
        _ = page.goto(f"{joined.url.rstrip('/')}/blocks.pdf")
        sheet = page.locator("#jamparty-pdf-page-1")
        expect(sheet).to_have_attribute("data-rendered", "true")
        box = sheet.bounding_box()
        assert box is not None
        x, y = box["x"], box["y"]
        assert box["width"] == pytest.approx(600, abs=1)
        overlay(page, "#edit").click()
        hover = sheet.locator(".pdf-block-hover")

        first_paragraph = "First paragraph starts here. This is the second line."
        cases = [
            (65, 42, 60, 402, 22, "Review notes"),
            (65, 86, 60, 346, 12, first_paragraph),
            (
                345,
                86,
                340,
                346,
                12,
                "Right column starts here. It has a second line too.",
            ),
            (270, 186, 265, 260, 12, "F = m times a"),
            (100, 300, 85, 130, 0, None),
            (65, 136, 60, 310, 12, "A separate paragraph."),
        ]
        for px, py, left, last_baseline, size, quote in cases:
            page.mouse.move(x + px, y + py)
            expect(hover).to_be_visible()
            outline = hover.bounding_box()
            assert outline is not None
            assert outline["x"] - x == pytest.approx(left, abs=0.5)
            page.mouse.click(x + px, y + py)
            send_comment(page, "Review this block.")
            [record] = recv_all(tmp_path)
            assert record.get("selectedText") == quote
            assert record["pdf"] == {
                "page": 1,
                "box": {
                    "x0": left,
                    "y0": pytest.approx(outline["y"] - y, abs=0.05),
                    "x1": pytest.approx(left + outline["width"], abs=0.05),
                    "y1": pytest.approx(450 - last_baseline + 0.22 * size, abs=1),
                },
            }
            if quote is None:
                assert outline["width"] == pytest.approx(75, abs=0.05)
                assert outline["height"] == pytest.approx(44, abs=0.05)

        page.mouse.move(x + 65, y + 86)
        expect(hover).to_be_visible()
        page.mouse.down()
        page.mouse.move(x + 85, y + 88, steps=5)
        expect(sheet.locator(".pdf-drag")).to_be_visible()
        page.mouse.up()
        send_comment(page, "Review a custom box.")
        [record] = recv_all(tmp_path)
        assert "selectedText" not in record
        assert record["pdf"] == {
            "page": 1,
            "box": {"x0": 65, "y0": 86, "x1": 85, "y1": 88},
        }

        page.mouse.move(x + 65, y + 86)
        expect(hover).to_be_visible()
        page.keyboard.press("Escape")
        expect(hover).to_have_count(0)
        assert page.locator("html").get_attribute("data-pdf-requested-tree") is None


def reading_pdf(pages: Sequence[tuple[int, int, list[str]]]) -> bytes:
    children = " ".join(f"{4 + 2 * i} 0 R" for i in range(len(pages)))
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        f"<< /Type /Pages /Kids [{children}] /Count {len(pages)} >>".encode(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    for i, (height, top, lines) in enumerate(pages):
        objects.append(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 {height}] "
            f"/Resources << /Font << /F1 3 0 R >> >> /Contents {5 + 2 * i} 0 R >>".encode()
        )
        commands = [
            f"BT /F1 18 Tf 72 {height - top - 30 * row} Td ({line}) Tj ET"
            for row, line in enumerate(lines)
        ]
        objects.append(pdf_stream(b"", "\n".join(commands).encode()))
    return make_pdf(objects)


@pytest.mark.parametrize("change", ["paragraph", "page", "delete", "scroll", "shrink"])
def test_pdf_rebuild_reading_position(tmp_path: Path, page: Page, change: str) -> None:
    site = tmp_path / "build"
    site.mkdir()
    pdf = site / "reading.pdf"
    title = "The telescope captured a remarkable image."
    chapter = ["Notes from a clear winter night.", title, "Several stars were visible."]
    cover = (792, 140, ["Cover sheet"])
    old_page = 20 if change == "shrink" else 2
    _ = pdf.write_bytes(reading_pdf([cover] * (old_page - 1) + [(792, 300, chapter)]))
    if change == "scroll":
        page.add_init_script("""(() => {
          const send = Worker.prototype.postMessage;
          Worker.prototype.postMessage = function (message, ...args) {
            if (message.action === "GetTextContent" &&
                document.documentElement.dataset.scrollOnSearch === "yes") {
              delete document.documentElement.dataset.scrollOnSearch;
              window.scrollBy(0, 120);
              document.documentElement.dataset.scrolledTo = String(window.scrollY);
            }
            return send.call(this, message, ...args);
          };
        })();""")

    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        page.set_viewport_size({"width": 848, "height": 720})
        _ = page.goto(f"{joined.url.rstrip('/')}/reading.pdf")
        scroll_to_page(page, old_page)
        sheet = page.locator(f"#jamparty-pdf-page-{old_page}")
        expect(sheet).to_have_attribute("data-rendered", "true")
        line = sheet.get_by_text(title, exact=True)
        _ = line.evaluate(
            "line => window.scrollBy(0, line.getBoundingClientRect().top - 20)"
        )
        before = line.bounding_box()
        assert before is not None
        scroll = float(page.evaluate("window.scrollY"))

        inserted = (930, 180, ["A new prefix changes the pagination.", title])
        revised_title = "The tele-scope captured a remarkable image."
        shifted = (960, 480, [chapter[0], revised_title, chapter[2]])
        updated = site / "reading.tmp"
        if change == "shrink":
            pages = [cover] * 5 + [(792, 300, chapter)] + [cover] * 4
        elif change == "delete":
            pages = [cover, (792, 450, [chapter[0], "The original line was removed."])]
        elif change == "paragraph":
            pages = [cover, (792, 360, ["An extra paragraph.", *chapter])]
        elif change == "page":
            pages = [cover, inserted, shifted]
        else:
            pages = [cover, (792, 480, ["An extra paragraph.", *chapter])]
            _ = page.evaluate("document.documentElement.dataset.scrollOnSearch = 'yes'")
        _ = updated.write_bytes(reading_pdf(pages))
        _ = updated.replace(pdf)

        if change in ("paragraph", "page"):
            destination = "3" if change == "page" else "2"
            moved = page.locator(f"#jamparty-pdf-page-{destination}").get_by_text(
                revised_title if change == "page" else title, exact=True
            )
            expect(moved).to_be_visible()
            expect(page.locator("#pdf-position")).to_have_text(
                f"{destination} / {len(pages)}"
            )
            after = moved.bounding_box()
            assert after is not None
            assert after["y"] == pytest.approx(before["y"], abs=2)
        elif change == "shrink":
            expect(page.locator(".pdf-page")).to_have_count(10)
            expect(page.locator("#jamparty-pdf-page-10")).to_have_attribute(
                "data-rendered", "true"
            )
            expect(page.locator("#pdf-position")).to_have_text("10 / 10")
        elif change == "delete":
            expect(sheet.get_by_text("The original line was removed.")).to_be_visible()
            assert float(page.evaluate("window.scrollY")) == pytest.approx(
                scroll, abs=2
            )
        else:
            expect(sheet.get_by_text("An extra paragraph.")).to_be_visible()
            scrolled_to = page.locator("html").get_attribute("data-scrolled-to")
            assert scrolled_to is not None
            assert float(scrolled_to) == pytest.approx(scroll + 120, abs=2)
            assert float(page.evaluate("window.scrollY")) == pytest.approx(
                float(scrolled_to), abs=2
            )


def test_single_pdf_opens_from_root_and_reloads(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "build"
    site.mkdir()
    pdf = site / "paper #%.pdf"
    _ = pdf.write_bytes(reading_pdf([(792, 150, ["Old PDF"])]))
    _ = (site / "paper.log").write_text("Private build log")
    with (
        share(tmp_path, "serve", str(pdf)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(page).to_have_url(f"{joined.url.rstrip('/')}/paper%20%23%25.pdf")
        text = page.locator("#jamparty-pdf-page-1 .textLayer")
        expect(text).to_have_text("Old PDF")
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        assert page.request.get(f"{joined.url.rstrip('/')}/paper.log").status == 404

        pdf.unlink()
        expect(page.locator("#pdf-status")).to_have_text(
            "Could not open the updated PDF."
        )
        expect(text).to_have_text("Old PDF")
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")

        updated = site / "paper.tmp"
        _ = updated.write_bytes(reading_pdf([(792, 150, ["Rebuilt PDF"])]))
        _ = updated.replace(pdf)
        expect(text).to_have_text("Rebuilt PDF")


def test_show_pdf_place_across_resize_and_rebuild(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "build"
    site.mkdir()
    path = site / "paper.pdf"
    _ = path.write_bytes(fixture_pdf("Original second page", outline_child="Appendix"))
    _ = (site / "index.html").write_text("<h1>Paper</h1>")
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        _ = page.goto(joined.url)
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        _ = jamparty(
            tmp_path,
            "send",
            "--at-pdf",
            "paper.pdf#page=2&box=72,120,350,150",
            "See the second page.",
        )
        general = overlay(page, "#agent-general .agent-message")
        general.get_by_role("link", name="Show").click()
        expect(page).to_have_url(f"{joined.url.rstrip('/')}/paper.pdf")
        sheet = page.locator("#jamparty-pdf-page-2")
        marker = sheet.locator(".pdf-answer-anchor").first
        expect(marker).to_be_in_viewport()
        expect(overlay(page, ".agent-message")).to_have_count(0)
        sections = overlay(page, "#pdf-outline")
        panel = overlay(page, "#pdf-sections")
        sections.click()
        expect(panel).to_be_visible()
        overlay(page, "#pill").click()
        expect(panel).to_be_hidden()
        earlier = overlay(page, "#answer-list .agent-reply")
        expect(earlier).to_contain_text("See the second page.")
        earlier.hover()
        expect(overlay(page, "#message-highlight")).to_be_visible()
        _ = page.evaluate("window.scrollTo(0, 0)")
        earlier.get_by_role("link", name="Show").click()
        expect(marker).to_be_in_viewport()
        expect(overlay(page, ".agent-message")).to_have_count(0)
        overlay(page, "#collapse").click()

        _ = jamparty(
            tmp_path,
            "send",
            "--at-pdf",
            "paper.pdf#page=2&box=72,120,350,150",
            "Live second-page note.",
        )
        answer = overlay(page, "#agent-anchored .agent-message")
        link_marker = overlay(page, ".agent-anchor-marker")
        leader = overlay(page, ".agent-anchor-line")
        expect(answer).to_contain_text("Live second-page note.")
        sections.click()
        expect(panel).to_be_visible()
        panel.get_by_role("button", name="Introduction").click()
        expect(panel).to_be_hidden()
        indicator = page.locator("#pdf-position")
        expect(indicator).to_have_text("1 / 2")
        expect(indicator).to_be_visible()
        expect(marker).not_to_be_in_viewport()
        expect(indicator).to_be_hidden()
        expect(answer).not_to_be_in_viewport()
        sections.click()
        panel.get_by_role("button", name="Analysis", exact=True).click()
        expect(marker).to_be_in_viewport()
        expect(answer).to_be_in_viewport()

        def check_position(*, side: str = "right", outer_margin: bool = False) -> None:
            expect(link_marker).to_have_attribute("data-side", side)
            expect(leader).to_have_attribute("d", re.compile("C"))
            sheet_box = sheet.bounding_box()
            mark_box = marker.bounding_box()
            answer_box = answer.bounding_box()
            assert (
                sheet_box is not None
                and mark_box is not None
                and answer_box is not None
            )
            scale = sheet_box["width"] / 612
            assert mark_box["x"] - sheet_box["x"] == pytest.approx(72 * scale, abs=1)
            assert mark_box["y"] - sheet_box["y"] == pytest.approx(120 * scale, abs=1)
            assert mark_box["width"] == pytest.approx(278 * scale, abs=1)
            assert link_marker.bounding_box() == pytest.approx(mark_box, abs=0.05)
            assert answer_box["x"] >= 8
            if side == "right":
                assert answer_box["y"] == pytest.approx(max(8, mark_box["y"]), abs=2)
                assert answer_box["x"] >= mark_box["x"] + mark_box["width"] + 12
            else:
                assert answer_box["y"] >= mark_box["y"] + mark_box["height"] + 12
            if outer_margin:
                assert answer_box["x"] >= sheet_box["x"] + sheet_box["width"] + 24

        check_position()
        page.set_viewport_size({"width": 1920, "height": 1080})
        expect(answer).to_have_css("width", "382.5px")
        check_position(outer_margin=True)
        page.set_viewport_size({"width": 848, "height": 720})
        expect(sheet).to_have_css("width", "816px")
        expect(sheet).to_have_attribute("data-rendered", "true")
        expect(marker).to_be_in_viewport()
        check_position()
        _ = page.evaluate("window.scrollBy(0, 80)")
        _ = page.wait_for_function("""() => {
            const mark = document.querySelector(".pdf-answer-anchor");
            const answer = document.querySelector("#jamparty")?.shadowRoot
                ?.querySelector("#agent-anchored .agent-message");
            return mark && answer &&
                Math.abs(mark.getBoundingClientRect().top - answer.getBoundingClientRect().top) < 2;
        }""")
        check_position()
        _ = answer.evaluate("(node) => node.blur()")
        page.mouse.move(0, 0)
        expect(link_marker).to_have_css("background-color", "rgba(0, 0, 0, 0)")
        answer.hover()
        expect(link_marker).to_have_css("background-color", "rgba(47, 128, 237, 0.08)")
        page.mouse.move(0, 0)
        page.set_viewport_size({"width": 390, "height": 844})
        expect(sheet).to_have_css("width", "358px")
        expect(marker).to_be_in_viewport()
        check_position(side="below")
        assert "system-ui" in answer.evaluate(
            "(node) => getComputedStyle(node).fontFamily"
        )

        new_pdf = site / "paper.tmp"
        _ = new_pdf.write_bytes(fixture_pdf("Rebuilt second page"))
        _ = new_pdf.replace(path)
        expect(sheet.locator(".textLayer")).to_have_text("Rebuilt second page")
        check_position(side="below")

        _ = new_pdf.write_bytes(blank_pdf(1))
        _ = new_pdf.replace(path)
        expect(sheet).to_have_count(0)
        expect(general).to_contain_text("Live second-page note.")
        expect(general).to_contain_text("Target unavailable")
        expect(general.get_by_role("link", name="Show")).to_have_count(0)
        expect(link_marker).to_have_count(0)
        expect(leader).to_have_count(0)
