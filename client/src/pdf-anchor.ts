import type { PdfAnchorFields } from "./generated/api";
import { isRecord } from "./json";

export type PdfAnchor = PdfAnchorFields;

export function parsePdfAnchor(value: unknown): PdfAnchor | undefined {
  if (!isRecord(value) || typeof value.page !== "number" ||
      !Number.isInteger(value.page) || value.page < 1 || value.page > 0xffffffff ||
      !isRecord(value.box)) return undefined;
  const { x0, y0, x1, y1 } = value.box;
  if (typeof x0 !== "number" || typeof y0 !== "number" ||
      typeof x1 !== "number" || typeof y1 !== "number" ||
      !(0 <= x0 && x0 < x1 && x1 <= 14400 && 0 <= y0 && y0 < y1 && y1 <= 14400)) {
    return undefined;
  }
  return { page: value.page, box: { x0, y0, x1, y1 } };
}
