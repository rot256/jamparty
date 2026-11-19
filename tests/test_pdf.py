"""The joiner's PDF viewer, anchors and live rebuilds."""

from base64 import b64decode
from pathlib import Path

import pytest
from playwright.sync_api import Page, Route, expect
from support import SocketTap, jamparty, join, overlay, recv_all, share, waiting


def fixture_pdf(label: str, padding: int = 0) -> bytes:
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
    return make_pdf(objects, padding)


def pdf_stream(fields: bytes, data: bytes) -> bytes:
    return (
        b"<< "
        + fields
        + f" /Length {len(data)} >>\nstream\n".encode()
        + data
        + b"\nendstream"
    )


def make_pdf(objects: list[bytes], padding: int = 0) -> bytes:
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
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(pdf)


def send_comment(page: Page, message: str) -> None:
    field = overlay(page, "textarea")
    expect(field).to_be_focused()
    field.fill(message)
    field.press("Enter")
    expect(overlay(page, "#jamparty-dialog")).not_to_be_visible()


def test_pdf_comments_and_rebuild(tmp_path: Path, page: Page) -> None:
    site = tmp_path / "build"
    site.mkdir()
    pdf = site / "paper.PDF"
    _ = pdf.write_bytes(fixture_pdf("Second page"))
    with (
        share(tmp_path, "serve", str(site)) as shared,
        join(tmp_path, shared) as joined,
    ):
        url = f"{joined.url.rstrip('/')}/paper.PDF?review=1"
        response = page.goto(url)
        assert response is not None
        assert "default-src 'none'" in response.headers["content-security-policy"]
        expect(overlay(page, "#connection")).to_have_attribute("data-state", "green")
        expect(page.locator("#jamparty-pdf-page-1")).to_have_attribute(
            "data-rendered", "true"
        )

        page.locator("#pdf-zoom").select_option("1")
        jump = page.locator("#pdf-page")
        jump.fill("2")
        jump.press("Tab")
        sheet = page.locator("#jamparty-pdf-page-2")
        expect(sheet).to_have_attribute("data-rendered", "true")
        expect(sheet.locator(".textLayer span")).to_have_text("Second page")
        # At 100%, 4/3 CSS pixels correspond to one PDF point.
        box = sheet.bounding_box()
        assert box is not None
        x, y = box["x"], box["y"]
        overlay(page, "#edit").click()
        page.mouse.move(x + 96, y + 256)
        page.mouse.down()
        page.mouse.move(x + 288, y + 384, steps=5)
        page.mouse.up()
        send_comment(page, "Move this figure.")
        overlay(page, "#edit").click()
        expect(sheet.locator(".pdf-comment")).to_have_count(1)
        [box_record] = recv_all(tmp_path)
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
        expect(sheet.locator(".pdf-comment")).to_have_count(2)
        _ = jamparty(tmp_path, "send", "--anchor", str(box_record["id"]), "Moved.")
        expect(overlay(page, ".agent-message.has-anchor")).to_contain_text("Moved.")

        _ = page.evaluate("window.scrollBy(0, 90)")
        scroll = float(page.evaluate("window.scrollY"))
        updated = site / "paper.tmp"
        _ = updated.write_bytes(fixture_pdf("Rebuilt page"))
        _ = updated.replace(pdf)
        expect(sheet.locator(".textLayer span")).to_have_text("Rebuilt page")
        expect(jump).to_have_value("2")
        expect(page.locator("#pdf-zoom")).to_have_value("1")
        assert float(page.evaluate("window.scrollY")) == pytest.approx(scroll, abs=2)
        expect(sheet.locator(".pdf-comment")).to_have_count(2)


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
        _ = page.goto(f"{joined.url.rstrip('/')}/rotated.pdf")
        page.locator("#pdf-zoom").select_option("1")
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
        expect(page.locator("#pdf-count")).to_have_text("of 100")
        requests = page.locator("html").get_attribute("data-pdf-pages")
        assert requests is not None and "89" not in requests.split(",")
        jump = page.locator("#pdf-page")
        jump.fill("90")
        jump.press("Tab")
        expect(page.locator("#jamparty-pdf-page-90")).to_have_attribute(
            "data-rendered", "true"
        )
        requests = page.locator("html").get_attribute("data-pdf-pages")
        assert requests is not None and "89" in requests.split(",")

        _ = page.evaluate("delete document.documentElement.dataset.pdfPages")
        updated = site / "long.tmp"
        _ = updated.write_bytes(blank_pdf(101))
        _ = updated.replace(site / "long.pdf")
        expect(page.locator("#pdf-count")).to_have_text("of 101")
        expect(page.locator("#jamparty-pdf-page-90")).to_have_attribute(
            "data-rendered", "true"
        )
        expect(jump).to_have_value("90")
        requests = page.locator("html").get_attribute("data-pdf-pages")
        assert requests is not None
        assert "89" in requests.split(",") and "49" not in requests.split(",")

        _ = updated.write_bytes(blank_pdf(1001))
        _ = updated.replace(site / "long.pdf")
        expect(page.locator("#pdf-status")).to_have_text(
            "This PDF exceeds the 1000-page viewer limit."
        )
        expect(page.locator("#pdf-count")).to_have_text("of 101")


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
        jump = page.locator("#pdf-page")
        root = page.locator("html")
        for number in (1, 6, 1):
            jump.fill(str(number))
            jump.press("Tab")
            sheet = page.locator(f"#jamparty-pdf-page-{number}")
            expect(sheet).to_have_attribute("data-rendered", "true")
            assert sheet.locator("canvas").evaluate(
                """canvas => Array.from(canvas.getContext('2d')
                    .getImageData(canvas.width / 2, canvas.height / 2, 1, 1).data)"""
            ) == [205, 80, 44, 255]
        evicted = (root.get_attribute("data-pdf-evicted") or "").split(",")
        assert {"1", "6"} <= set(evicted)
        renders = (root.get_attribute("data-pdf-renders") or "").split(",")
        assert renders.count("0") >= 2
