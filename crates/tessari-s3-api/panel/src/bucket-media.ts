//! A bucket's media rules, inline under its row: anyone who reads the bucket
//! sees them, and whoever configures it changes them — the server decides which
//! one the signed-in key is. The rules are sent whole, so a repeated save
//! changes nothing; saving none turns media processing off.

import { call } from "./api.ts";
import { announce, el, field, fill, mono } from "./dom.ts";
import { OUTPUTS, readMediaRules, typesOf, type MediaRule } from "./media-model.ts";
import { ignored, type Bucket } from "./models.ts";
import { refusal, type Screen } from "./screen.ts";

/** One rule's controls, and how to read them back into what the server takes. */
type Editor = { readonly box: HTMLFieldSetElement; readonly read: () => Readonly<Record<string, unknown>> };

let made = 0;

function ruleEditor(base: string, rule: MediaRule | null, remove: (editor: Editor) => void): Editor {
  made += 1;
  const id = `${base}-${made}`;
  const name = field(`${id}-id`, "Rule ID", { spellcheck: "false", autocomplete: "off", required: "" });
  const types = field(`${id}-types`, "Content types", { spellcheck: "false", autocomplete: "off", required: "" }, "Comma-separated, such as image/* or video/mp4.");
  const prefix = field(`${id}-prefix`, "Key prefix", { spellcheck: "false", autocomplete: "off" }, "Empty: every key in the bucket.");
  const boxes = OUTPUTS.map((output) => {
    const box = el("input", { id: `${id}-${output.name}`, type: "checkbox", value: output.name });
    box.checked = rule?.outputs.includes(output.name) ?? false;
    return { output, box };
  });
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
    el("fieldset", { class: "checks" }, el("legend", {}, "Outputs"), ...boxes.map(({ output, box: check }) => el("label", { class: "check", for: check.id }, check, output.label))),
    el("div", { class: "actions" }, drop),
  );
  const editor: Editor = {
    box,
    read: () => ({
      id: name.input.value.trim(),
      content_types: typesOf(types.input.value),
      prefix: prefix.input.value,
      outputs: boxes.filter(({ box: check }) => check.checked).map(({ output }) => output.name),
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
    el("p", { class: "muted" }, "An upload whose content type and key match a rule gets the rule's renditions, written under the key's .renditions/ folder. Without rules nothing is processed."),
    body,
    said,
  );
  form.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      close();
    }
  });
  const route = `/buckets/${encodeURIComponent(bucket.name)}/media`;
  void call("GET", route, readMediaRules).then((answer) => {
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
    answer.value.forEach(append);
    show();
    const reason = field(`${base}-reason`, "Reason (optional)", { maxlength: "500" }, "Recorded with your key id.");
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    cancel.addEventListener("click", close);
    fill(body, none, list, el("div", { class: "actions" }, add), reason.row, el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Save rules"), cancel));
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const why = reason.input.value.trim();
      const rules = editors.map((editor) => editor.read());
      const saved = await call("PUT", route, ignored, why === "" ? { rules } : { rules, reason: why });
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
