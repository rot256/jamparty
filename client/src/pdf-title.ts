function cleanTitle(value: unknown): string {
  if (typeof value !== "string") return "";
  return value.replace(/(?!\s)[\p{Cc}\p{Bidi_Control}]/gu, "").replace(/\s+/g, " ").trim()
    .replace(/^(.{199}).{2}.*$/u, "$1…");
}

/** The Info title, else the XMP title (PDF.js joins its language
 *  alternatives into one string), else the file name. */
export function choosePdfTitle(pathname: string, infoTitle?: unknown, xmpTitle?: unknown): string {
  const title = cleanTitle(infoTitle) || cleanTitle(xmpTitle);
  if (title) return title;

  const fileName = pathname.split("/").pop() ?? "";
  let decoded = fileName;
  try {
    decoded = decodeURIComponent(fileName);
  } catch {
    // A malformed escape should still leave a readable title.
  }
  return cleanTitle(decoded) || "PDF viewer";
}
