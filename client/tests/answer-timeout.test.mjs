import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import test from "node:test";
import { build } from "esbuild";

const { outputFiles: [module] } = await build({
  entryPoints: ["client/src/answer-timeout.ts"], bundle: true, format: "esm", write: false,
});
const { ANSWER_TIMEOUT_PRESETS, answerTimeoutLabel, answerTimeoutPreset } = await import(
  `data:text/javascript;base64,${Buffer.from(module.contents).toString("base64")}`
);

test("answer timeout presets map to their labels and numeric settings", () => {
  assert.deepEqual(ANSWER_TIMEOUT_PRESETS.map(answerTimeoutLabel), [
    "Never", "10 s", "30 s", "1 min", "5 min",
  ]);
  for (const seconds of [0, 10, 30, 60, 300]) {
    assert.equal(answerTimeoutPreset(String(seconds)), seconds);
  }
});

test("custom durations have labels and cannot be sent by the preset buttons", () => {
  assert.equal(answerTimeoutLabel(45), "45 s");
  assert.equal(answerTimeoutLabel(120), "2 min");
  assert.equal(answerTimeoutLabel(90), "90 s");
  assert.equal(answerTimeoutLabel(125), "125 s");
  assert.equal(answerTimeoutLabel(3599), "3599 s");
  assert.equal(answerTimeoutLabel(3600), "60 min");
  for (const value of ["45", "120", "3601", "", " 10", undefined]) {
    assert.equal(answerTimeoutPreset(value), undefined);
  }
});
