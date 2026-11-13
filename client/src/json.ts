// JSON read back from sessionStorage or the joiner is checked before use.

/** Whether `value` is an object whose fields can be read. */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
