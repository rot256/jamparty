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

export class UploadError extends Error {
  constructor(message: string, readonly transient: boolean) {
    super(message);
  }
}

export async function upload(file: File): Promise<Attachment> {
  const reply = await fetch(UPLOAD_URL, {
    method: "POST",
    mode: "same-origin",
    redirect: "error",
    body: file,
  }).catch(() => null);
  if (reply === null) throw new UploadError("Could not reach the joiner.", true);
  let result: unknown;
  try {
    result = JSON.parse(await reply.text());
  } catch (error) {
    if (!(error instanceof SyntaxError)) throw new UploadError("The upload reply was interrupted.", true);
  }
  if (reply.status === 201 && isRecord(result) && isResourceId(result.id)) {
    return { type: "file", file: result.id, name: file.name };
  }
  const reason = isRecord(result) && typeof result.error === "string" ? result.error : undefined;
  const transient = [408, 429, 502, 503, 504].includes(reply.status) ||
    reason === "could not read the attachment";
  throw new UploadError(reason ?? "Could not upload the attachment.", transient);
}
