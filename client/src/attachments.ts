// Upload bytes separately from the page socket. The joiner checks the
// share's content address before returning it.

import { UPLOAD_URL } from "./config";
import { type Attachment, MAX_ATTACHMENTS } from "./generated/api";
import { isRecord } from "./json";

export function parseAttachments(value: unknown): Attachment[] | null {
  if (!Array.isArray(value) || value.length > MAX_ATTACHMENTS) return null;
  const files: Attachment[] = [];
  for (const entry of value) {
    if (!isRecord(entry) || entry.type !== "file" || !isResourceId(entry.file) ||
        typeof entry.name !== "string") return null;
    files.push({ type: "file", file: entry.file, name: entry.name });
  }
  return files;
}

function isResourceId(value: unknown): value is string {
  return typeof value === "string" && value.length === 69 && /^sha3:[0-9a-f]{64}$/.test(value);
}

export async function upload(file: File): Promise<Attachment> {
  const reply = await fetch(UPLOAD_URL, {
    method: "POST",
    mode: "same-origin",
    redirect: "error",
    body: file,
  }).catch(() => null);
  if (reply !== null) {
    const result: unknown = await reply.json().catch(() => null);
    if (reply.status === 201 && isRecord(result) && isResourceId(result.id)) {
      return { type: "file", file: result.id, name: file.name };
    }
    if (!reply.ok && isRecord(result) && typeof result.error === "string") {
      throw new Error(result.error);
    }
  }
  throw new Error("Could not upload the attachment. Try again.");
}
