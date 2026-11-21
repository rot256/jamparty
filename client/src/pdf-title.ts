function cleanTitle(value: unknown): string {
  if (typeof value !== "string") return "";
  return value.replace(/(?!\s)[\p{Cc}\p{Bidi_Control}]/gu, "").replace(/\s+/g, " ").trim()
    .replace(/^(.{199}).{2}.*$/u, "$1…");
}

export function choosePdfTitle(pathname: string, xmpTitle?: unknown, infoTitle?: unknown): string {
  const title = cleanTitle(xmpTitle) || cleanTitle(infoTitle);
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
