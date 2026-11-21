import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import test from "node:test";
import { build } from "esbuild";

const { outputFiles: [module] } = await build({
  entryPoints: ["client/src/pdf-title.ts"], bundle: true, format: "esm", write: false,
});
const { choosePdfTitle } = await import(
  `data:text/javascript;base64,${Buffer.from(module.contents).toString("base64")}`
);

test("prefers a normalized Info title over the XMP title", () => {
  assert.equal(choosePdfTitle("/paper.pdf", "  Research\n\t \u0000 notes ", "ResearchRecherche"), "Research notes");
});

test("uses the XMP title when Info is empty or unusable", () => {
  assert.equal(choosePdfTitle("/paper.pdf", " \u0000\t ", "  XMP   title "), "XMP title");
  assert.equal(choosePdfTitle("/paper.pdf", null, "XMP"), "XMP");
});

test("strips controls and caps titles without splitting Unicode code points", () => {
  assert.equal(choosePdfTitle("/paper.pdf", "Pa\u0000p\u007fer\u202e"), "Paper");
  assert.equal(choosePdfTitle("/paper.pdf", "📄".repeat(200)), "📄".repeat(200));
  assert.equal(choosePdfTitle("/paper.pdf", "📄".repeat(201)), "📄".repeat(199) + "…");
});

test("falls back to the decoded filename", () => {
  assert.equal(choosePdfTitle("/shared/paper%20%23%25.pdf", "", "   "), "paper #%.pdf");
  assert.equal(choosePdfTitle("/paper%ZZ.pdf"), "paper%ZZ.pdf");
});
