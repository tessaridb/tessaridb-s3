//! A key's row actions in the listing, as icon buttons: its details beside the
//! list, a download (recorded by the server), the key copied, and a delete that
//! only goes through while the object is still the version listed — the listed
//! ETag is the condition, so a rewrite in between is refused.

import { call, path, search } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, mono } from "./dom.ts";
import { iconAction } from "./icon-action.ts";
import { ignored, type ObjectRow } from "./models.ts";
import { deleteRefusal } from "./object-delete.ts";
import { open, settle } from "./row-region.ts";
import type { Screen } from "./screen.ts";

/** The buttons for `object`'s row `line`; `details` opens the side pane and `gone` drops the row's selection. */
export function objectActions(screen: Screen, bucket: string, object: ObjectRow, line: HTMLTableRowElement, details: () => void, gone: () => void): HTMLButtonElement[] {
  const where = `/buckets/${encodeURIComponent(bucket)}/object`;
  const info = iconAction("info", "Details", `Details of ${object.key}`);
  const download = iconAction("download", "Download", `Download ${object.key}`);
  const copy = iconAction("copy", "Copy key", `Copy the key ${object.key}`);
  const remove = iconAction("trash", "Delete…", `Delete ${object.key}`, "danger");
  info.addEventListener("click", details);
  download.addEventListener("click", () => {
    location.assign(`${path(`${where}/content`)}${search([["key", object.key]])}`);
  });
  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(object.key);
      announce(`Copied the key ${object.key}.`);
    } catch {
      announce("Copying is not allowed here; open the object's page and copy the key by hand.");
    }
  });
  remove.addEventListener("click", () => {
    const close = open(
      line,
      confirmation(
        `delete-${line.rowIndex}`,
        mono(`${bucket}/${object.key}`),
        `There is no versioning: the object is gone for every client. It is deleted only if it is still ETag ${object.etag}.`,
        "Delete object",
        async (reason) => {
          const done = await call("DELETE", `${where}${search([["key", object.key]])}`, ignored, { etag: object.etag, reason });
          if (!done.ok) {
            const shown = settle(screen, done);
            return shown === null ? null : deleteRefusal(shown);
          }
          close();
          // The pressed button goes with its row; the keyboard carries on from the row beside it.
          const beside = (line.nextElementSibling ?? line.previousElementSibling)?.querySelector<HTMLElement>("a.name");
          line.remove();
          gone();
          beside?.focus();
          announce(`Deleted ${object.key} from ${bucket}.`);
          return null;
        },
        () => {
          close();
          remove.focus();
        },
      ),
    );
  });
  return [info, download, copy, remove];
}
