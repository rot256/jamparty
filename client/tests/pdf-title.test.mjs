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

test("prefers a normalized XMP title over the Info title", () => {
  assert.equal(choosePdfTitle("/paper.pdf", "  Research\n\t \u0000 notes ", "Info"), "Research notes");
});

test("uses the Info title when XMP is empty or unusable", () => {
  assert.equal(choosePdfTitle("/paper.pdf", " \u0000\t ", "  Info   title "), "Info title");
  assert.equal(choosePdfTitle("/paper.pdf", null, "Info"), "Info");
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
