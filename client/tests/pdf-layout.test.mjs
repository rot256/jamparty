import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import test from "node:test";
import { build } from "esbuild";

const { outputFiles: [module] } = await build({
  entryPoints: ["client/src/pdf-layout.ts"], bundle: true, format: "esm", write: false,
});
const { linesFor, paragraphsFor } = await import(
  `data:text/javascript;base64,${Buffer.from(module.contents).toString("base64")}`
);

function run(x, baseline, width, text, size = 12) {
  return {
    bounds: [x, baseline - 2, x + width, baseline + 8],
    baseline, size, text, horizontal: true, rtl: false,
  };
}

function group(...runs) {
  return paragraphsFor(linesFor(runs), [0, 0, 600, 500]);
}

test("groups split runs and interleaved columns independently", () => {
  assert.deepEqual(group(
    run(320, 450, 100, "right"), run(60, 450, 30, "first"),
    run(94, 449, 50, "words"), run(320, 436, 95, "column"),
    run(60, 436, 90, "next line"),
  ), [
    { bounds: [60, 434, 150, 458], text: "first words next line" },
    { bounds: [320, 434, 420, 458], text: "right column" },
  ]);
});

test("splits at a heading, indentation, a large gap and a display line", () => {
  assert.deepEqual(group(
    run(60, 450, 150, "Heading", 18), run(60, 433, 200, "Body"),
    run(60, 420, 200, "continues"), run(76, 407, 210, "Indented"),
    run(76, 378, 180, "After a gap"), run(265, 365, 70, "x + y"),
    run(76, 352, 160, "Next paragraph"),
  ).map(({ text }) => text), [
    "Heading", "Body continues", "Indented", "After a gap", "x + y", "Next paragraph",
  ]);
});
