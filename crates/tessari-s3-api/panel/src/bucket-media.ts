//! A bucket's media rules, inline under its row: anyone who reads the bucket
//! sees them, and whoever configures it changes them — the server decides which
//! one the signed-in key is. The rules are sent whole, so a repeated save
//! changes nothing; saving none turns media processing off.

import { call } from "./api.ts";
import { announce, el, field, fill, mono, segmented } from "./dom.ts";
import { isKind, KINDS, outputOf, readMediaSet, readSweep, spell, sweepText, typesOf, type MediaRule, type Output } from "./media-model.ts";
import { ignored, type Bucket } from "./models.ts";
import { refusal, type Screen } from "./screen.ts";

/** One rule's controls, and how to read them back into what the server takes. */
type Editor = { readonly box: HTMLFieldSetElement; readonly read: () => Readonly<Record<string, unknown>> };

let made = 0;

/** The box modes a size written as `WxH` takes. */
const MODES = ["fit", "cover", "exact"] as const;

/** One output's choices, each a row of options rather than a popup: the kind, the format and a video's size from the
 *  server's lists; an image's or a poster's size written as a side or a box, the box's mode picked beside it; a
 *  stream's ladder written as `360p+720p`; and an optional quality. The options follow the kind. */
function outputRow(id: string, spelled: string | null, remove: (row: HTMLElement) => void): { readonly row: HTMLElement; readonly read: () => string } {
  const fallback: Output = { kind: "image", size: "1280", format: "webp", quality: "" };
  const start = (spelled === null ? null : outputOf(spelled)) ?? fallback;
  const kind = segmented(`${id}-kind`, "Kind", Object.keys(KINDS));
  kind.choose(start.kind);
  const lines = segmented(`${id}-lines`, "Video size", KINDS.video.sizes);
  lines.choose(start.kind === "video" && start.format !== "hls" ? start.size : "720p");
  const [dimensions = "", mode = "fit"] = start.kind === "video" ? ["", "fit"] : start.size.split("-");
  const box = segmented(`${id}-mode`, "Box mode", MODES);
  box.choose(mode);
  const written = (suffix: string, label: string, hint: string): HTMLInputElement =>
    el("input", { id: `${id}-${suffix}`, "aria-label": label, placeholder: hint, spellcheck: "false", autocomplete: "off", size: "14" });
  const side = written("size", "Size: a side such as 1280 or a box such as 400x400", "1280 or 400x400");
  side.value = start.kind === "video" ? (start.format === "hls" ? start.size : "360p+720p") : dimensions;
  const quality = written("quality", "Quality, 1-100, for JPEG, WebP or AVIF", "quality");
  quality.value = start.quality;
  const formats = el("span", {});
  let format = segmented(`${id}-format`, "Format", KINDS[start.kind].formats);
  format.choose(start.format);
  formats.append(format.group);
  const chosenKind = (): keyof typeof KINDS => {
    const picked = kind.chosen();
    return isKind(picked) ? picked : "image";
  };
  const follow = (): void => {
    const video = chosenKind() === "video";
    const stream = video && format.chosen() === "hls";
    lines.group.hidden = !video || stream;
    side.hidden = video && !stream;
    side.placeholder = stream ? "360p+720p+1080p" : "1280 or 400x400";
    box.group.hidden = video || !side.value.includes("x");
    quality.hidden = video || format.chosen() === "png";
  };
  kind.group.addEventListener("change", () => {
    const keep = format.chosen();
    format = segmented(`${id}-format`, "Format", KINDS[chosenKind()].formats);
    format.choose(keep);
    if (format.chosen() === "") {
      format.choose(KINDS[chosenKind()].formats[0] ?? "");
    }
    formats.replaceChildren(format.group);
    follow();
  });
  formats.addEventListener("change", follow);
  side.addEventListener("input", follow);
  follow();
  const drop = el("button", { type: "button", class: "quiet", "aria-label": "Remove output" }, "Remove");
  const row = el("div", { class: "actions" }, kind.group, lines.group, side, box.group, formats, quality, drop);
  drop.addEventListener("click", () => remove(row));
  const read = (): string => {
    const picked = chosenKind();
    const video = picked === "video";
    const stream = video && format.chosen() === "hls";
    const typed = side.value.trim();
    const size = video && !stream ? lines.chosen() : !video && typed.includes("x") && box.chosen() !== "fit" ? `${typed}-${box.chosen()}` : typed;
    const named = video || quality.hidden ? "" : quality.value.trim();
    return `${picked}/${size}/${format.chosen()}${named === "" ? "" : `@${named}`}`;
  };
  return { row, read };
}

function ruleEditor(base: string, rule: MediaRule | null, remove: (editor: Editor) => void): Editor {
  made += 1;
  const id = `${base}-${made}`;
  const name = field(`${id}-id`, "Rule ID", { spellcheck: "false", autocomplete: "off", required: "" });
  const types = field(`${id}-types`, "Content types", { spellcheck: "false", autocomplete: "off", required: "" }, "Comma-separated, such as image/* or video/mp4.");
  const prefix = field(`${id}-prefix`, "Key prefix", { spellcheck: "false", autocomplete: "off" }, "Empty: every key in the bucket.");
  const rows: Array<{ readonly row: HTMLElement; readonly read: () => string }> = [];
  const outputs = el("div", {});
  const removeOutput = (row: HTMLElement): void => {
    const index = rows.findIndex((entry) => entry.row === row);
    if (index >= 0) {
      rows.splice(index, 1);
    }
    row.remove();
    addOutput.focus();
  };
  const appendOutput = (spelled: string | null): void => {
    made += 1;
    const entry = outputRow(`${id}-out-${made}`, spelled, removeOutput);
    rows.push(entry);
    outputs.append(entry.row);
  };
  const addOutput = el("button", { type: "button", class: "quiet" }, "Add output");
  addOutput.addEventListener("click", () => {
    appendOutput(null);
    rows.at(-1)?.row.querySelector("input")?.focus();
  });
  (rule?.outputs ?? [spell({ kind: "image", size: "320", format: "jpeg", quality: "" })]).forEach(appendOutput);
  name.input.value = rule?.id ?? "";
  types.input.value = rule?.content_types.join(", ") ?? "";
  prefix.input.value = rule?.prefix ?? "";
  const drop = el("button", { type: "button", class: "quiet" }, "Remove rule");
  const box = el(
    "fieldset",
    { class: "card" },
    el("legend", {}, "Rule"),
    name.row,
    types.row,
    prefix.row,
    el("fieldset", { class: "checks" }, el("legend", {}, "Outputs: kind, size, format"), outputs, addOutput),
    el("div", { class: "actions" }, drop),
  );
  const editor: Editor = {
    box,
    read: () => ({
      id: name.input.value.trim(),
      content_types: typesOf(types.input.value),
      prefix: prefix.input.value,
      outputs: rows.map((entry) => entry.read()),
    }),
  };
  drop.addEventListener("click", () => remove(editor));
  return editor;
}

/** The media form for `bucket`, as a region `open` places under its row; `close` removes it. */
export function mediaForm(screen: Screen, bucket: Bucket, close: () => void): HTMLElement {
  const base = `media-${bucket.name}`;
  const said = el("div", { "aria-live": "polite" });
  const body = el("div", {}, el("p", { class: "muted" }, "Reading the media rules…"));
  const form = el(
    "form",
    { class: "confirm", novalidate: "", "aria-labelledby": `${base}-title` },
    el("p", { id: `${base}-title` }, el("strong", {}, "Media: "), mono(bucket.name)),
    el("p", { class: "muted" }, "An upload whose content type and key match a rule gets the rule's renditions, written under the key's .renditions/ folder. An image size is the long side in pixels (1280) or a box (400x400) fitted, covered or exact, and JPEG, WebP or AVIF may name a quality; video sizes are the shorter side, and an HLS stream names its ladder (360p+720p). Without rules nothing is processed."),
    body,
    said,
  );
  form.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      close();
    }
  });
  const route = `/buckets/${encodeURIComponent(bucket.name)}/media`;
  void call("GET", route, readMediaSet).then((answer) => {
    if (!answer.ok) {
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      const back = el("button", { type: "button", class: "quiet" }, "Close");
      back.addEventListener("click", close);
      fill(body, refusal(answer), el("div", { class: "actions" }, back));
      back.focus();
      return;
    }
    const editors: Editor[] = [];
    const list = el("div", {});
    const none = el("p", { class: "muted" }, "No rules: nothing in this bucket is processed.");
    const show = (): void => {
      none.hidden = editors.length > 0;
    };
    const remove = (editor: Editor): void => {
      editors.splice(editors.indexOf(editor), 1);
      editor.box.remove();
      show();
      add.focus();
    };
    const append = (rule: MediaRule | null): Editor => {
      const editor = ruleEditor(base, rule, remove);
      editors.push(editor);
      list.append(editor.box);
      show();
      return editor;
    };
    const add = el("button", { type: "button", class: "quiet" }, "Add rule");
    add.addEventListener("click", () => append(null).box.querySelector("input")?.focus());
    answer.value.rules.forEach(append);
    const version = answer.value.version;
    show();
    const reason = field(`${base}-reason`, "Reason (optional)", { maxlength: "500" }, "Recorded with your key id.");
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    cancel.addEventListener("click", close);
    const swept = el("p", { class: "muted", "aria-live": "polite" }, "Reading how far the objects already here are rendered…");
    const sweep = el("button", { type: "button", class: "quiet" }, "Render existing objects");
    const showSweep = async (): Promise<void> => {
      const seen = await call("GET", `${route}/sweep`, readSweep);
      swept.textContent = seen.ok ? sweepText(seen.value) : "How far the objects already here are rendered could not be read.";
    };
    sweep.addEventListener("click", async () => {
      const why = reason.input.value.trim();
      const started = await call("POST", `${route}/sweep`, readSweep, why === "" ? {} : { reason: why });
      if (started.ok) {
        announce(`Rendering the objects already in ${bucket.name} by its saved rules.`);
        await showSweep();
        return;
      }
      if (started.status === 401) {
        screen.signIn();
        return;
      }
      fill(said, refusal(started));
    });
    void showSweep();
    fill(body, none, list, el("div", { class: "actions" }, add), reason.row, el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Save rules"), cancel), swept, el("div", { class: "actions" }, sweep));
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const why = reason.input.value.trim();
      const rules = editors.map((editor) => editor.read());
      const saved = await call("PUT", route, ignored, why === "" ? { rules, version } : { rules, version, reason: why });
      if (saved.ok) {
        announce(`Saved the media rules of ${bucket.name}.`);
        screen.redraw();
        return;
      }
      if (saved.status === 401) {
        screen.signIn();
        return;
      }
      fill(said, refusal(saved));
    });
    (editors[0]?.box.querySelector("input") ?? add).focus();
  });
  return form;
}
