import type { PlaceAnchor } from "./generated/api";
import { isRecord } from "./json";
import { parsePdfAnchor } from "./pdf-anchor";

// Check the encoded path and its segments before it can reach a link.
function validPath(path: string): boolean {
  if (path.length > 4096 || !path.startsWith("/") || path.startsWith("//") ||
      /[^\x21-\x7e]|["\\?#]/u.test(path)) return false;
  try {
    const decoded = decodeURIComponent(path);
    return new URL(path, "http://localhost").pathname === path &&
      !decoded.startsWith("//") && !decoded.includes("\\") &&
      !Array.from(decoded).some((character) => /\p{Cc}/u.test(character)) &&
      !decoded.split("/").some((part) => part === "." || part === "..") &&
      decoded.split("/")[1] !== "_jamparty";
  } catch {
    return false;
  }
}

export function parsePlaceAnchor(value: unknown): PlaceAnchor | undefined {
  if (!isRecord(value) || Object.keys(value).length !== 3 ||
      typeof value.path !== "string" || !validPath(value.path)) return undefined;
  const { path } = value;
  if (value.type === "html" && typeof value.selector === "string" &&
      value.selector.trim() !== "" && Array.from(value.selector).length <= 2048) {
    return { type: "html", path, selector: value.selector };
  }
  if (value.type === "pdf" && path.toLowerCase().endsWith(".pdf") &&
      isRecord(value.pdf) && Object.keys(value.pdf).length === 2 &&
      isRecord(value.pdf.box) && Object.keys(value.pdf.box).length === 4) {
    const pdf = parsePdfAnchor(value.pdf);
    if (pdf !== undefined) return { type: "pdf", path, pdf };
  }
  return undefined;
}

/** A share selector is only ever interpreted by the DOM selector engine. */
export function placeElement(root: Pick<Document, "querySelector">, selector: string): Element | null {
  try {
    return root.querySelector(selector);
  } catch {
    return null;
  }
}
