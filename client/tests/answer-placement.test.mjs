import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import test from "node:test";
import { build } from "esbuild";

const { outputFiles: [module] } = await build({
  entryPoints: ["client/src/answer-placement.ts"], bundle: true, format: "esm", write: false,
});
const { placeAnswer } = await import(
  `data:text/javascript;base64,${Buffer.from(module.contents).toString("base64")}`
);

const overlaps = (a, b) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;

test("a card never covers a target that overflows its page", () => {
  const view = { left: 8, top: 8, right: 1272, bottom: 892 };
  const page = { left: 0, top: 0, right: 600, bottom: 800 };
  const target = { left: 580, top: 100, right: 700, bottom: 140 };
  const placement = placeAnswer(() => 80, view, target, page, []);
  assert.ok(placement !== undefined);
  assert.ok(!overlaps(placement.box, target), JSON.stringify(placement.box));
});

test("a card beside a target inside its page goes to the page gutter", () => {
  const view = { left: 8, top: 8, right: 1912, bottom: 1072 };
  const page = { left: 480, top: 12, right: 1440, bottom: 1250 };
  const target = { left: 593, top: 500, right: 1327, bottom: 565 };
  const placement = placeAnswer(() => 80, view, target, page, []);
  assert.ok(placement !== undefined);
  assert.ok(placement.box.left >= page.right + 24, JSON.stringify(placement.box));
});
