import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import test from "node:test";
import { build } from "esbuild";

const { outputFiles: [module] } = await build({
  entryPoints: ["client/src/answer-anchor.ts"], bundle: true, format: "esm", write: false,
});
const { parsePlaceAnchor, placeElement } = await import(
  `data:text/javascript;base64,${Buffer.from(module.contents).toString("base64")}`
);

test("place anchors cannot make links outside the share", () => {
  for (const path of [
    "https://other.example/", "javascript:alert(1)", "//other.example/", "/\\other.example/",
    "/%2fother.example/", "/%5cother.example/", "/%2e%2e/_jamparty/api/",
    "/_jamparty/api/", "/%5fjamparty/api/", "/a?next=//other.example/",
    "/x\n", "/x#anchor", "/bad%", "/" + "p".repeat(4096),
  ]) {
    assert.equal(parsePlaceAnchor({ type: "html", path, selector: "#intro" }), undefined, path);
  }
  const anchor = { type: "html", path: "/design%20notes.html", selector: "#intro" };
  assert.deepEqual(parsePlaceAnchor(anchor), anchor);
  assert.equal(parsePlaceAnchor({ ...anchor, extra: 1 }), undefined);
  assert.equal(parsePlaceAnchor({ ...anchor, selector: "s".repeat(2049) }), undefined);
});

test("selectors are only passed to querySelector and syntax errors are contained", () => {
  const selector = '<img src=x onerror="globalThis.injected = true">';
  const anchor = parsePlaceAnchor({ type: "html", path: "/", selector });
  assert.equal(anchor.selector, selector);
  const root = {
    querySelector(value) {
      assert.equal(value, selector);
      throw new SyntaxError("invalid selector");
    },
  };
  assert.equal(placeElement(root, anchor.selector), null);
  assert.equal(globalThis.injected, undefined);
});

test("PDF answers require valid geometry and a PDF path", () => {
  const box = { x0: 72, y0: 140, x1: 540, y1: 180 };
  const anchor = { type: "pdf", path: "/paper.pdf", pdf: { page: 2, box } };
  assert.deepEqual(parsePlaceAnchor(anchor), anchor);
  assert.equal(parsePlaceAnchor({ ...anchor, path: "/paper.html" }), undefined);
  assert.equal(parsePlaceAnchor({ ...anchor, pdf: { ...anchor.pdf, page: 0 } }), undefined);
  assert.equal(parsePlaceAnchor({ ...anchor, pdf: { page: 2, box: { ...box, y1: 100 } } }), undefined);
});
