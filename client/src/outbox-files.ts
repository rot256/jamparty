// Keep file bytes until acceptance or discard. Acceptance receipts let
// another tab with the same sessionStorage entry recognize the comment.
import { MAX_OUTBOX } from "./config";
import { type Attachment, MAX_ATTACHMENTS, MAX_FILE_BYTES } from "./generated/api";

const FILES = "files";
const ACCEPTED = "accepted";
const MAX_AGE = 7 * 24 * 60 * 60 * 1000;

interface SavedFiles {
  at: number;
  files: File[];
}

interface Acceptance {
  id: string;
  attachments: Attachment[];
}

interface Receipt extends Acceptance {
  at: number;
}

async function transaction<T>(
  mode: IDBTransactionMode,
  work: (tx: IDBTransaction, finish: (result: T) => void) => void,
): Promise<T> {
  const db = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open("jamparty-outbox", 2);
    request.onupgradeneeded = (event) => {
      if (event.oldVersion === 0) {
        request.result.createObjectStore(FILES).createIndex("at", "at");
      }
      request.result.createObjectStore(ACCEPTED).createIndex("at", "at");
    };
    request.onsuccess = () => { resolve(request.result); };
    request.onerror = () => { reject(new Error("Could not open attachment storage.", { cause: request.error })); };
  });
  try {
    return await new Promise<T>((resolve, reject) => {
      const tx = db.transaction([FILES, ACCEPTED], mode);
      let result: T;
      tx.oncomplete = () => { resolve(result); };
      tx.onabort = () => { reject(new Error("Attachment storage transaction aborted.", { cause: tx.error })); };
      work(tx, (value) => { result = value; });
    });
  } finally {
    db.close();
  }
}

function valid(files: unknown): files is File[] {
  return Array.isArray(files) && files.length > 0 && files.length <= MAX_ATTACHMENTS &&
    files.every((file) => file instanceof File && file.size <= MAX_FILE_BYTES);
}

function prune(store: IDBObjectStore, done: () => void): void {
  const expired = store.index("at").openCursor(IDBKeyRange.upperBound(Date.now() - MAX_AGE));
  expired.onsuccess = () => {
    const cursor = expired.result;
    if (cursor === null) {
      done();
    } else {
      cursor.delete();
      cursor.continue();
    }
  };
}

export function pruneFiles(): void {
  void transaction("readwrite", (tx, finish) => {
    prune(tx.objectStore(FILES), () => { finish(undefined); });
    prune(tx.objectStore(ACCEPTED), () => { finish(undefined); });
  }).catch(() => undefined);
}

export function keepFiles(ref: string, files: File[], replaces?: string): Promise<boolean> {
  if (!valid(files)) return Promise.resolve(false);
  return transaction("readwrite", (tx, finish) => {
    const store = tx.objectStore(FILES);
    prune(store, () => {
      const count = store.count();
      // Keep the original until the replacement's session entry is saved.
      const replaced = replaces === undefined ? null : store.getKey(replaces);
      const check = (): void => {
        if (count.result - Number(replaced?.result !== undefined) >= MAX_OUTBOX) {
          finish(false);
          return;
        }
        store.put({ at: Date.now(), files } satisfies SavedFiles, ref);
        finish(true);
      };
      if (replaced === null) count.onsuccess = check;
      else replaced.onsuccess = check;
    });
  });
}

export function readFiles(ref: string): Promise<{ files: File[]; accepted?: Acceptance }> {
  return transaction<{ files: File[]; accepted?: Acceptance }>("readonly", (tx, finish) => {
    const saved = tx.objectStore(FILES).get(ref);
    const receipt = tx.objectStore(ACCEPTED).get(ref);
    receipt.onsuccess = () => {
      const record = saved.result as SavedFiles | undefined;
      const accepted = receipt.result as Receipt | undefined;
      finish(accepted === undefined ? { files: valid(record?.files) ? record.files : [] } :
        { files: [], accepted });
    };
  });
}

export function forgetFiles(ref: string, accepted?: Acceptance): Promise<void> {
  return transaction<undefined>("readwrite", (tx, finish) => {
    if (accepted !== undefined) {
      tx.objectStore(ACCEPTED).put({ at: Date.now(), ...accepted } satisfies Receipt, ref);
    }
    tx.objectStore(FILES).delete(ref);
    finish(undefined);
  }).catch(() => {
    // Leftover records are pruned on a later page load or save.
  });
}
