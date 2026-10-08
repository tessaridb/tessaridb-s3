"use strict";
(() => {
  // src/models.ts
  //! What the console API answers, and the guards that check an answer has that
  //! shape before anything reads it. The API is ours, but the page is the boundary
  //! where its bytes become values, so nothing is cast — each reader returns the
  //! typed value or null.
  var record = (value) => typeof value === "object" && value !== null && !Array.isArray(value);
  var text = (value) => typeof value === "string";
  var count = (value) => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
  var textOrNull = (value) => value === null || text(value);
  var countOrNull = (value) => value === null || count(value);
  function list(value, item) {
    if (!Array.isArray(value)) {
      return null;
    }
    const items = [];
    for (const entry of value) {
      const read = item(entry);
      if (read === null) {
        return null;
      }
      items.push(read);
    }
    return items;
  }
  function strings(value) {
    if (!record(value)) {
      return null;
    }
    const out = {};
    for (const [name, entry] of Object.entries(value)) {
      if (!text(entry)) {
        return null;
      }
      out[name] = entry;
    }
    return out;
  }
  function drive(v) {
    if (v === null) {
      return null;
    }
    return record(v) && count(v["capacity"]) && count(v["free"]) && count(v["available"]) ? { capacity: v["capacity"], free: v["free"], available: v["available"] } : void 0;
  }
  function member(v) {
    if (!record(v) || !text(v["node"]) || !text(v["endpoint"]) || typeof v["answering"] !== "boolean") {
      return null;
    }
    const space = drive(v["drive"]);
    return space === void 0 ? null : { node: v["node"], endpoint: v["endpoint"], answering: v["answering"], drive: space };
  }
  function readStatus(v) {
    if (!record(v) || !text(v["version"]) || !textOrNull(v["node"]) || !text(v["region"]) || !textOrNull(v["erasure"])) {
      return null;
    }
    const members = v["members"] === null ? null : list(v["members"], member);
    const backlog = v["heal_backlog"];
    const heal = backlog === null ? null : record(backlog) && count(backlog["listed"]) && typeof backlog["more"] === "boolean" ? { listed: backlog["listed"], more: backlog["more"] } : void 0;
    const space = drive(v["drive"]);
    if (v["members"] !== null && members === null || heal === void 0 || space === void 0) {
      return null;
    }
    return { version: v["version"], node: v["node"], region: v["region"], erasure: v["erasure"], members, heal_backlog: heal, drive: space };
  }
  var bucketUsage = (v) => record(v) && text(v["bucket"]) && count(v["objects"]) && count(v["bytes"]) ? { bucket: v["bucket"], objects: v["objects"], bytes: v["bytes"] } : null;
  function readUsage(v) {
    if (!record(v) || !textOrNull(v["taken"]) || !count(v["objects"]) || !count(v["bytes"])) {
      return null;
    }
    const buckets2 = list(v["buckets"], bucketUsage);
    return buckets2 === null ? null : { taken: v["taken"], buckets: buckets2, objects: v["objects"], bytes: v["bytes"] };
  }
  var bucket = (v) => record(v) && text(v["name"]) && text(v["created"]) && text(v["region"]) ? { name: v["name"], created: v["created"], region: v["region"] } : null;
  function readBuckets(v) {
    return record(v) ? list(v["buckets"], bucket) : null;
  }
  var objectRow = (v) => record(v) && text(v["key"]) && count(v["size"]) && text(v["etag"]) && text(v["modified"]) ? { key: v["key"], size: v["size"], etag: v["etag"], modified: v["modified"] } : null;
  function readListing(v) {
    if (!record(v) || !textOrNull(v["next"])) {
      return null;
    }
    const objects2 = list(v["objects"], objectRow);
    const prefixes = list(v["prefixes"], (entry) => text(entry) ? entry : null);
    return objects2 === null || prefixes === null ? null : { objects: objects2, prefixes, next: v["next"] };
  }
  function readDetail(v) {
    if (!record(v) || !text(v["key"]) || !count(v["size"]) || !text(v["etag"]) || !text(v["modified"]) || !countOrNull(v["parts"])) {
      return null;
    }
    const headers = strings(v["headers"]);
    const metadata = strings(v["metadata"]);
    const checksums = strings(v["checksums"]);
    if (headers === null || metadata === null || checksums === null) {
      return null;
    }
    return { key: v["key"], size: v["size"], etag: v["etag"], modified: v["modified"], headers, metadata, checksums, parts: v["parts"] };
  }
  var action = (v) => record(v) && count(v["position"]) && text(v["at"]) && text(v["operator"]) && text(v["operation"]) && text(v["target"]) && textOrNull(v["reason"]) && text(v["outcome"]) ? {
    position: v["position"],
    at: v["at"],
    operator: v["operator"],
    operation: v["operation"],
    target: v["target"],
    reason: v["reason"],
    outcome: v["outcome"]
  } : null;
  function readActions(v) {
    if (!record(v) || !countOrNull(v["next"])) {
      return null;
    }
    const actions2 = list(v["actions"], action);
    return actions2 === null ? null : { actions: actions2, next: v["next"] };
  }
  function readProblem(v) {
    return record(v) && text(v["code"]) && text(v["message"]) ? { code: v["code"], message: v["message"] } : null;
  }
  var ignored = (_v) => true;

  // src/api.ts
  //! The one way the page talks to the server: same-origin JSON under /api/v1.
  //! The session is an HttpOnly cookie the page never sees; a change carries a
  //! JSON body, which a cross-site form cannot send. Every answer is checked
  //! against its reader before it is used.
  var API = "/api/v1";
  var path = (route) => `${API}${route}`;
  function search(pairs2) {
    const kept = pairs2.filter((pair) => pair[1] !== null);
    return kept.length === 0 ? "" : `?${kept.map(([name, value]) => `${name}=${encodeURIComponent(String(value))}`).join("&")}`;
  }
  async function call(method, route, read, body) {
    let response;
    try {
      response = await fetch(path(route), {
        method,
        credentials: "same-origin",
        headers: body === void 0 ? { accept: "application/json" } : { accept: "application/json", "content-type": "application/json" },
        ...body === void 0 ? {} : { body: JSON.stringify(body) }
      });
    } catch {
      return { ok: false, status: 0, code: "network", message: "The console did not answer. Check that the node is running, then try again." };
    }
    let parsed = null;
    if (response.status !== 204) {
      try {
        parsed = await response.json();
      } catch {
        parsed = void 0;
      }
    }
    if (response.ok) {
      const value = parsed === void 0 ? null : read(parsed);
      return value === null ? { ok: false, status: response.status, code: "bad_answer", message: "The server answered in a shape this page does not understand. Reload the page." } : { ok: true, value };
    }
    const problem = readProblem(parsed);
    return {
      ok: false,
      status: response.status,
      code: problem?.code ?? "unknown",
      message: problem?.message ?? `The server refused the request (status ${response.status}).`
    };
  }

  // src/dom.ts
  //! Building the page. Everything the server hands over — bucket names, keys,
  //! reasons, metadata — is S3 data that any client could have written, so it is
  //! only ever placed with `textContent` or as an attribute value; nothing here
  //! parses a string as markup.
  function el(tag, attributes = {}, ...children) {
    const made = document.createElement(tag);
    for (const [name, value] of Object.entries(attributes)) {
      made.setAttribute(name, value);
    }
    for (const child of children) {
      if (child !== null && child !== false) {
        made.append(child);
      }
    }
    return made;
  }
  function at(id) {
    const found = document.getElementById(id);
    if (found === null) {
      throw new Error(`the page has no element #${id}`);
    }
    return found;
  }
  function fill(parent2, ...children) {
    parent2.replaceChildren(...children.filter((child) => child !== null && child !== false));
  }
  function announce(words) {
    at("status-line").textContent = words;
  }
  function field(id, label, attributes = {}, hint) {
    const input = el("input", { id, name: id, type: "text", ...attributes });
    const described = hint === void 0 ? null : el("p", { id: `${id}-hint`, class: "hint" }, hint);
    if (described !== null) {
      input.setAttribute("aria-describedby", described.id);
    }
    return { row: el("div", { class: "field" }, el("label", { for: id }, label), input, described), input };
  }
  function table(caption, head2, rows) {
    return el(
      "div",
      { class: "scroll" },
      el(
        "table",
        {},
        el("caption", { class: "sr-only" }, caption),
        el("thead", {}, el("tr", {}, ...head2.map((name) => el("th", { scope: "col" }, name)))),
        el("tbody", {}, ...rows)
      )
    );
  }
  var row = (...cells) => el("tr", {}, ...cells.map((cell) => el("td", {}, cell)));
  var mono = (text2) => el("code", {}, text2);
  var all = (selector) => Array.from(document.querySelectorAll(selector));

  // src/format.ts
  //! How figures are written: sizes in binary units, times in the reader's locale.
  var UNITS = ["KiB", "MiB", "GiB", "TiB", "PiB"];
  var whole = new Intl.NumberFormat("en", { maximumFractionDigits: 0 });
  var tenth = new Intl.NumberFormat("en", { minimumFractionDigits: 1, maximumFractionDigits: 1 });
  function size(bytes) {
    if (bytes < 1024) {
      return `${whole.format(bytes)} B`;
    }
    let value = bytes / 1024;
    let unit = 0;
    while (value >= 1024 && unit < UNITS.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${tenth.format(value)} ${UNITS[unit] ?? "PiB"}`;
  }
  var stamp = new Intl.DateTimeFormat(void 0, { dateStyle: "medium", timeStyle: "medium" });
  function moment(iso) {
    const at2 = new Date(iso);
    return Number.isNaN(at2.getTime()) ? iso : stamp.format(at2);
  }
  var percent = new Intl.NumberFormat("en", { style: "percent", maximumFractionDigits: 0 });
  var share = (part, whole2) => percent.format(whole2 === 0 ? 0 : part / whole2);
  var amount = (value) => value.toLocaleString();

  // src/route.ts
  //! The console's views as hash routes, so every view is a URL an operator can
  //! reload or hand to a colleague. Keys and prefixes are S3 data and may hold any
  //! character, so every value is percent-encoded on the way out and decoded on
  //! the way in; a hash that does not name a view is the overview, never an error.
  var OVERVIEW = { kind: "status" };
  function query(pairs2) {
    const kept = pairs2.filter((pair) => pair[1] !== null);
    return kept.length === 0 ? "" : `?${kept.map(([name, value]) => `${name}=${encodeURIComponent(value)}`).join("&")}`;
  }
  function format(route) {
    switch (route.kind) {
      case "status":
        return "#/";
      case "buckets":
        return "#/buckets";
      case "objects":
        return `#/b/${encodeURIComponent(route.bucket)}${query([
          ["prefix", route.prefix === "" ? null : route.prefix],
          ["cursor", route.cursor]
        ])}`;
      case "object":
        return `#/o/${encodeURIComponent(route.bucket)}${query([["key", route.key]])}`;
      case "actions":
        return `#/actions${query([["before", route.before === null ? null : String(route.before)]])}`;
    }
  }
  function position(text2) {
    if (text2 === null || !/^[1-9][0-9]*$/.test(text2)) {
      return null;
    }
    const value = Number(text2);
    return Number.isSafeInteger(value) ? value : null;
  }
  function parse(hash) {
    const text2 = hash.startsWith("#") ? hash.slice(1) : hash;
    const mark2 = text2.indexOf("?");
    const path2 = mark2 === -1 ? text2 : text2.slice(0, mark2);
    const params = new URLSearchParams(mark2 === -1 ? "" : text2.slice(mark2 + 1));
    try {
      if (path2 === "/buckets") {
        return { kind: "buckets" };
      }
      if (path2 === "/actions") {
        return { kind: "actions", before: position(params.get("before")) };
      }
      if (path2.startsWith("/b/")) {
        const bucket2 = decodeURIComponent(path2.slice(3));
        return bucket2 === "" ? { kind: "buckets" } : { kind: "objects", bucket: bucket2, prefix: params.get("prefix") ?? "", cursor: params.get("cursor") };
      }
      if (path2.startsWith("/o/")) {
        const bucket2 = decodeURIComponent(path2.slice(3));
        const key = params.get("key");
        return bucket2 === "" || key === null || key === "" ? OVERVIEW : { kind: "object", bucket: bucket2, key };
      }
    } catch {
    }
    return OVERVIEW;
  }

  // src/screen.ts
  //! What every view is handed: the region it draws in, a way to know it is still
  //! the view on screen (a slow answer must not paint over a newer one), and the
  //! one place a refusal is turned into words — so a 401 anywhere leads to signing
  //! in, and every other failure says what happened and what to do next.
  function next(failure) {
    switch (failure.code) {
      case "rate_limit":
        return "Wait a minute, then try again.";
      case "unavailable":
      case "network":
        return "Try again in a moment.";
      case "precondition_failed":
        return "Reload it and check it before acting.";
      case "action_not_recorded":
        return "The action was carried out. Tell whoever audits this node; the server log holds the record.";
      default:
        return failure.status >= 500 ? "Try again; if it keeps failing, check the server log." : "";
    }
  }
  function refusal(failure) {
    const advice = next(failure);
    return el("p", { class: "message bad", role: "alert" }, el("strong", {}, "Not done. "), failure.message, advice === "" ? null : ` ${advice}`);
  }
  function failed(screen, title, failure) {
    if (!screen.live()) {
      return;
    }
    if (failure.status === 401) {
      screen.signIn();
      return;
    }
    const retry = el("button", { type: "button" }, "Try again");
    retry.addEventListener("click", screen.redraw);
    fill(screen.main, head(title), el("div", { class: "card" }, refusal(failure), retry));
  }
  var heading = (title) => el("h1", { tabindex: "-1", class: "view-title" }, title);
  var head = (title, lede, action2) => el("div", { class: "page-head" }, el("div", {}, heading(title), lede === void 0 || lede === null ? null : el("p", { class: "lede" }, lede)), action2 ?? null);
  function loading(screen, title, what) {
    fill(screen.main, head(title), el("div", { class: "card", "aria-busy": "true" }, el("p", { class: "muted" }, `Loading ${what}…`)));
  }
  var empty = (why, action2) => el("div", { class: "empty" }, el("p", {}, why), action2 ?? null);

  // src/actions.ts
  //! The action record, newest first: who did what to which bucket or object,
  //! when, why, and how it ended. Paged by position; there is no total.
  var TITLE = "Action record";
  var PAGE = 50;
  async function actions(screen, before) {
    loading(screen, TITLE, "the action record");
    const answer = await call("GET", `/actions${search([["before", before], ["limit", PAGE]])}`, readActions);
    if (!answer.ok) {
      failed(screen, TITLE, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const page = answer.value;
    const rows = page.actions.map(
      (action2) => row(
        moment(action2.at),
        mono(action2.operator),
        action2.operation.replaceAll("_", " "),
        mono(action2.target),
        action2.reason ?? el("span", { class: "muted" }, "none given"),
        el("span", { class: `chip ${action2.outcome === "done" || action2.outcome === "sent" ? "ok" : "warn"}` }, action2.outcome.replaceAll("_", " "))
      )
    );
    fill(
      screen.main,
      head(TITLE, "Every change and every download made through this console, kept for a year and never edited."),
      el(
        "section",
        { class: "card flush" },
        rows.length === 0 ? empty(before === null ? "No console actions are recorded yet." : "No older actions.") : table("Console actions, newest first", ["When", "Operator", "Operation", "Target", "Reason", "Outcome"], rows),
        before === null && page.next === null ? null : el(
          "p",
          { class: "paging" },
          before === null ? null : el("a", { href: format({ kind: "actions", before: null }) }, "Newest"),
          before !== null && page.next !== null ? " · " : null,
          page.next === null ? null : el("a", { href: format({ kind: "actions", before: page.next }) }, "Older")
        )
      )
    );
  }

  // src/confirm.ts
  //! The confirmation for an irreversible change to one thing: it names the thing,
  //! says what cannot be undone, and will not proceed without a reason — the
  //! reason goes into the action record beside the operator's key id.
  var REASON_MAX = 500;
  function confirmation(id, what, consequence, button, act, cancel) {
    const reason = el("textarea", { id: `${id}-reason`, rows: "2", maxlength: String(REASON_MAX), required: "", "aria-describedby": `${id}-reason-hint` });
    const hint = el("p", { id: `${id}-reason-hint`, class: "hint" }, "Required. Recorded with your key id in the action record.");
    const go = el("button", { type: "submit", class: "danger" }, button);
    const back = el("button", { type: "button" }, "Cancel");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", "aria-labelledby": `${id}-title`, novalidate: "" },
      el("p", { id: `${id}-title` }, el("strong", {}, `${button}: `), what),
      el("p", { class: "muted" }, consequence),
      el("div", { class: "field" }, el("label", { for: reason.id }, "Reason"), reason, hint),
      el("div", { class: "actions" }, go, back),
      said
    );
    back.addEventListener("click", cancel);
    form.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        cancel();
      }
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const why = reason.value.trim();
      if (why === "") {
        reason.setAttribute("aria-invalid", "true");
        fill(said, el("p", { class: "message bad", role: "alert" }, "Say why — a reason is required for this action."));
        reason.focus();
        return;
      }
      reason.removeAttribute("aria-invalid");
      go.disabled = true;
      const failure = await act(why);
      go.disabled = false;
      if (failure !== null) {
        fill(said, refusal(failure));
      }
    });
    queueMicrotask(() => reason.focus());
    return form;
  }

  // src/icons.ts
  //! The console's icons: one outline family on a 24px grid — stroke 2, round
  //! caps and joins, `currentColor` so they follow the theme and the state they
  //! sit in. Every icon here sits beside a text label, so each is hidden from
  //! screen readers; an icon never carries meaning on its own.
  var SVG = "http://www.w3.org/2000/svg";
  var box = (x, y, w, h, r) => `M${x + r} ${y}h${w - 2 * r}a${r} ${r} 0 0 1 ${r} ${r}v${h - 2 * r}a${r} ${r} 0 0 1 -${r} ${r}h-${w - 2 * r}a${r} ${r} 0 0 1 -${r} -${r}v-${h - 2 * r}a${r} ${r} 0 0 1 ${r} -${r}z`;
  var ring = (cx, cy, r) => `M${cx - r} ${cy}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 -${2 * r} 0`;
  var SHAPES = {
    overview: [box(3, 3, 7, 7, 1.5), box(14, 3, 7, 7, 1.5), box(3, 14, 7, 7, 1.5), box(14, 14, 7, 7, 1.5)],
    buckets: ["M4 6c0 1.66 3.58 3 8 3s8-1.34 8-3-3.58-3-8-3-8 1.34-8 3z", "M4 6v12c0 1.66 3.58 3 8 3s8-1.34 8-3V6", "M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"],
    record: ["M9 6h11", "M9 12h11", "M9 18h11", "M4.5 6h.01", "M4.5 12h.01", "M4.5 18h.01"],
    sun: [ring(12, 12, 4), "M12 2.5v2", "M12 19.5v2", "M2.5 12h2", "M19.5 12h2", "M5.3 5.3l1.4 1.4", "M17.3 17.3l1.4 1.4", "M5.3 18.7l1.4-1.4", "M17.3 6.7l1.4-1.4"],
    moon: ["M20 14.5A8.5 8.5 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"],
    "sign-out": ["M10 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h4", "M15 8l4 4-4 4", "M19 12H9"],
    folder: ["M3 7.5a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"],
    file: ["M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z", "M14 3v5h5"],
    download: ["M12 4v11", "M7.5 10.5L12 15l4.5-4.5", "M5 20h14"],
    trash: ["M4 7h16", "M9.5 7V4.5h5V7", "M6.5 7l1 13h9l1-13", "M10 11v5.5", "M14 11v5.5"],
    plus: ["M12 5v14", "M5 12h14"],
    chevron: ["M9.5 6l6 6-6 6"],
    node: [box(3, 4, 18, 7, 2), box(3, 13, 18, 7, 2), "M7 7.5h.01", "M7 16.5h.01"],
    members: [ring(6, 7, 2.5), ring(18, 7, 2.5), ring(12, 18, 2.5), "M8.5 7h7", "M7.3 9.2l3.4 6.6", "M16.7 9.2l-3.4 6.6"],
    layers: ["M12 3l9 5-9 5-9-5z", "M3 13l9 5 9-5"],
    pulse: ["M3 12h4l3-7 4 14 3-7h4"],
    back: ["M14.5 6l-6 6 6 6"],
    disk: [box(3, 5, 18, 14, 2.5), "M3 13h18", "M16.5 16h.01"]
  };
  function icon(name) {
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("class", "icon");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("focusable", "false");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "2");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    for (const d of SHAPES[name]) {
      const path2 = document.createElementNS(SVG, "path");
      path2.setAttribute("d", d);
      svg.append(path2);
    }
    return svg;
  }

  // src/buckets.ts
  //! Buckets: find one (the list is complete, so filtering it here hides nothing
  //! the server sent), see how many objects and bytes each holds as last measured,
  //! create one from the form the header's button reveals, and delete an empty one
  //! with a reason.
  var TITLE2 = "Buckets";
  function createForm(screen, opener) {
    const name = field("new-bucket", "Bucket name", { spellcheck: "false", autocomplete: "off", required: "" }, "3-63 lowercase letters, digits, dots and hyphens.");
    const reason = field("new-bucket-reason", "Reason (optional)", { maxlength: "500" });
    const said = el("div", { "aria-live": "polite" });
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    const form = el(
      "form",
      { class: "card", novalidate: "", hidden: "", "aria-labelledby": "new-bucket-title" },
      el("h2", { id: "new-bucket-title" }, "New bucket"),
      name.row,
      reason.row,
      el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Create bucket"), cancel),
      said
    );
    const close = () => {
      form.hidden = true;
      opener.setAttribute("aria-expanded", "false");
      opener.focus();
    };
    opener.addEventListener("click", () => {
      form.hidden = false;
      opener.setAttribute("aria-expanded", "true");
      name.input.focus();
    });
    cancel.addEventListener("click", close);
    form.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        close();
      }
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const wanted = name.input.value.trim();
      const why = reason.input.value.trim();
      const answer = await call("POST", "/buckets", ignored, why === "" ? { name: wanted } : { name: wanted, reason: why });
      if (!answer.ok) {
        if (answer.status === 401) {
          screen.signIn();
          return;
        }
        name.input.setAttribute("aria-invalid", "true");
        fill(said, refusal(answer));
        return;
      }
      announce(`Created bucket ${wanted}.`);
      screen.redraw();
    });
    return form;
  }
  function figures(usage, bucket2) {
    if (!usage.ok || usage.value.taken === null || usage.value.taken < bucket2.created) {
      return null;
    }
    return usage.value.buckets.find((entry) => entry.bucket === bucket2.name) ?? { bucket: bucket2.name, objects: 0, bytes: 0 };
  }
  var numeric = (text2) => el("span", { class: "numeric" }, text2);
  function bucketRow(screen, bucket2, usage) {
    const remove = el("button", { type: "button", class: "quiet" }, icon("trash"), "Delete…");
    const held = figures(usage, bucket2);
    const line = row(
      el("a", { class: "name", href: format({ kind: "objects", bucket: bucket2.name, prefix: "", cursor: null }) }, icon("buckets"), mono(bucket2.name)),
      numeric(held === null ? "—" : amount(held.objects)),
      numeric(held === null ? "—" : size(held.bytes)),
      moment(bucket2.created),
      mono(bucket2.region),
      remove
    );
    remove.addEventListener("click", () => {
      const cell = el("td", { colspan: "6" });
      const ask = el("tr", { class: "asking" }, cell);
      const close = () => {
        ask.remove();
        remove.focus();
      };
      cell.append(
        confirmation(
          `delete-${bucket2.name}`,
          mono(bucket2.name),
          "Only an empty bucket can be deleted, and the name becomes free for anyone to take.",
          "Delete bucket",
          async (reason) => {
            const answer = await call("DELETE", `/buckets/${encodeURIComponent(bucket2.name)}`, ignored, { reason });
            if (answer.ok) {
              announce(`Deleted bucket ${bucket2.name}.`);
              screen.redraw();
              return null;
            }
            if (answer.status === 401) {
              screen.signIn();
              return null;
            }
            return answer;
          },
          close
        )
      );
      line.after(ask);
    });
    return line;
  }
  function measuredLine(count2, usage) {
    const listed = `${count2.toLocaleString()} on this cluster`;
    if (!usage.ok) {
      return `${listed} · sizes could not be read`;
    }
    return usage.value.taken === null ? `${listed} · sizes not measured yet` : `${listed} · sizes measured ${moment(usage.value.taken)}`;
  }
  async function buckets(screen) {
    loading(screen, TITLE2, "buckets");
    const [answer, usage] = await Promise.all([call("GET", "/buckets", readBuckets), call("GET", "/usage", readUsage)]);
    if (!answer.ok) {
      failed(screen, TITLE2, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const all2 = answer.value;
    const opener = el("button", { type: "button", class: "primary", "aria-expanded": "false", "aria-controls": "new-bucket-form" }, icon("plus"), "New bucket");
    const form = createForm(screen, opener);
    form.id = "new-bucket-form";
    const filter = field("bucket-filter", "Filter by name", { type: "search", spellcheck: "false", autocomplete: "off" });
    const listed = el("div", {});
    const draw2 = () => {
      const wanted = filter.input.value.trim();
      const shown = all2.filter((bucket2) => bucket2.name.includes(wanted));
      fill(
        listed,
        all2.length === 0 ? empty("No buckets yet. Create one with “New bucket”, or with any S3 client.") : shown.length === 0 ? empty(`No bucket name contains “${wanted}”.`) : table("Buckets", ["Name", "Objects", "Size", "Created", "Region", "Actions"], shown.map((bucket2) => bucketRow(screen, bucket2, usage)))
      );
    };
    filter.input.addEventListener("input", draw2);
    draw2();
    fill(
      screen.main,
      head(TITLE2, measuredLine(all2.length, usage), opener),
      form,
      el("section", { class: "card flush" }, all2.length === 0 ? null : el("div", { class: "toolbar" }, filter.row), listed)
    );
  }

  // src/object.ts
  //! One object: what the server holds for it, a recorded download, and a delete
  //! that only goes through while the object is still the version shown here —
  //! the ETag on screen is the condition, so a rewrite in between is refused.
  var TITLE3 = "Object";
  function pairs(caption, values) {
    const entries = Object.entries(values);
    return entries.length === 0 ? el("div", { class: "empty" }, el("p", {}, "None.")) : table(caption, ["Name", "Value"], entries.map(([name, value]) => row(mono(name), mono(value))));
  }
  var parent = (key) => key.slice(0, key.lastIndexOf("/") + 1);
  async function object(screen, bucket2, key) {
    loading(screen, TITLE3, "the object");
    const where = `/buckets/${encodeURIComponent(bucket2)}/object`;
    const answer = await call("GET", `${where}${search([["key", key]])}`, readDetail);
    if (!answer.ok) {
      failed(screen, TITLE3, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const detail = answer.value;
    const back = format({ kind: "objects", bucket: bucket2, prefix: parent(key), cursor: null });
    const reason = field("download-reason", "Reason (optional)", { maxlength: "500" }, "Downloads are recorded with your key id.");
    const download = el("a", { class: "button", download: "" }, icon("download"), "Download");
    const point = () => {
      const why = reason.input.value.trim();
      download.setAttribute("href", `${path(`${where}/content`)}${search([["key", key], ["reason", why === "" ? null : why]])}`);
    };
    reason.input.addEventListener("input", point);
    point();
    const remove = el("button", { type: "button", class: "danger" }, icon("trash"), "Delete object…");
    const asking = el("div", {});
    remove.addEventListener("click", () => {
      remove.hidden = true;
      fill(
        asking,
        confirmation(
          "delete-object",
          mono(`${bucket2}/${key}`),
          `There is no versioning: the object is gone for every client. It is deleted only if it is still ETag ${detail.etag}.`,
          "Delete object",
          async (why) => {
            const done = await call("DELETE", `${where}${search([["key", key]])}`, ignored, { etag: detail.etag, reason: why });
            if (done.ok) {
              announce(`Deleted ${key} from ${bucket2}.`);
              location.hash = back;
              return null;
            }
            if (done.status === 401) {
              screen.signIn();
              return null;
            }
            return done.status === 412 ? { ...done, message: "This object changed since you opened it, so it was not deleted." } : done;
          },
          () => {
            fill(asking);
            remove.hidden = false;
            remove.focus();
          }
        )
      );
    });
    const name = key.slice(key.lastIndexOf("/") + 1) || key;
    fill(
      screen.main,
      head(name, el("span", {}, el("a", { class: "back", href: back }, icon("back"), "Back to the listing"), " · ", mono(`${bucket2}/${key}`))),
      el(
        "section",
        { class: "card" },
        el(
          "dl",
          { class: "facts" },
          el("dt", {}, "Size"),
          el("dd", {}, `${size(detail.size)} (${detail.size.toLocaleString()} bytes)`),
          el("dt", {}, "ETag"),
          el("dd", {}, mono(detail.etag)),
          el("dt", {}, "Last modified"),
          el("dd", {}, moment(detail.modified)),
          el("dt", {}, "Upload"),
          el("dd", {}, detail.parts === null ? "Single request" : `Multipart, ${detail.parts} parts`)
        )
      ),
      el("section", { class: "card" }, el("h2", {}, "Actions"), reason.row, el("div", { class: "actions" }, download, remove), asking),
      el("section", { class: "card flush" }, el("h2", {}, "Headers"), pairs("Stored headers", detail.headers)),
      el("section", { class: "card flush" }, el("h2", {}, "User metadata"), pairs("User metadata", detail.metadata)),
      el("section", { class: "card flush" }, el("h2", {}, "Checksums"), pairs("Stored checksums", detail.checksums))
    );
  }

  // src/objects.ts
  //! A bucket's keys a page at a time, in the server's byte order, rolled up at
  //! `/` like a folder tree. Paging is by the server's cursor alone: there is no
  //! total to show, because counting a large bucket is a scan.
  var PAGE2 = 100;
  function trail(bucket2, prefix) {
    const steps = [
      el("a", { href: format({ kind: "buckets" }) }, "Buckets"),
      icon("chevron")
    ];
    const parts = prefix.split("/").filter((piece) => piece !== "");
    const step = (label, at2, last) => last ? el("span", { "aria-current": "page" }, label) : el("a", { href: format({ kind: "objects", bucket: bucket2, prefix: at2, cursor: null }) }, label);
    steps.push(step(bucket2, "", parts.length === 0));
    let walked = "";
    parts.forEach((part, index) => {
      walked += `${part}/`;
      steps.push(icon("chevron"), step(part, walked, index === parts.length - 1));
    });
    return el("nav", { class: "trail", "aria-label": "Prefix" }, ...steps);
  }
  async function objects(screen, bucket2, prefix, cursor) {
    const title = bucket2;
    loading(screen, title, "keys");
    const answer = await call(
      "GET",
      `/buckets/${encodeURIComponent(bucket2)}/objects${search([["prefix", prefix === "" ? null : prefix], ["delimiter", "/"], ["cursor", cursor], ["limit", PAGE2]])}`,
      readListing
    );
    if (!answer.ok) {
      failed(screen, title, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const page = answer.value;
    const folders = page.prefixes.map(
      (folder) => row(el("a", { class: "name", href: format({ kind: "objects", bucket: bucket2, prefix: folder, cursor: null }) }, icon("folder"), mono(folder.slice(prefix.length))), "Folder", "", "")
    );
    const files = page.objects.map(
      (object2) => row(
        el("a", { class: "name", href: format({ kind: "object", bucket: bucket2, key: object2.key }) }, icon("file"), mono(object2.key.slice(prefix.length))),
        el("span", { title: `${object2.size.toLocaleString()} bytes` }, size(object2.size)),
        moment(object2.modified),
        mono(object2.etag)
      )
    );
    const nothing = cursor !== null ? empty("No more keys on this page.", el("a", { href: format({ kind: "objects", bucket: bucket2, prefix, cursor: null }) }, "Back to the first page")) : prefix === "" ? empty("This bucket is empty.") : empty("Nothing under this prefix.", el("a", { href: format({ kind: "objects", bucket: bucket2, prefix: "", cursor: null }) }, "Back to the top of the bucket"));
    const paging = cursor === null && page.next === null ? null : el(
      "p",
      { class: "paging" },
      cursor === null ? null : el("a", { href: format({ kind: "objects", bucket: bucket2, prefix, cursor: null }) }, "First page"),
      cursor !== null && page.next !== null ? " · " : null,
      page.next === null ? null : el("a", { href: format({ kind: "objects", bucket: bucket2, prefix, cursor: page.next }) }, "Next page")
    );
    fill(
      screen.main,
      head(title, trail(bucket2, prefix)),
      el(
        "section",
        { class: "card flush" },
        folders.length + files.length === 0 ? nothing : table(`Keys under ${prefix === "" ? "the bucket" : prefix}`, ["Name", "Size", "Last modified", "ETag"], [...folders, ...files]),
        paging
      )
    );
  }

  // src/signin.ts
  //! Signing in with the node's root access key. The secret goes to the server
  //! once; what the browser keeps is an HttpOnly session cookie it cannot read,
  //! and the fields are cleared as soon as the answer arrives.
  function signIn(main2, done) {
    const key = field("access-key", "Access key ID", { autocomplete: "username", spellcheck: "false", required: "" });
    const secret = field("secret-key", "Secret access key", { type: "password", autocomplete: "current-password", required: "" });
    const submit = el("button", { type: "submit", class: "primary" }, "Sign in");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "card", novalidate: "" },
      el("img", { class: "logo", src: "/favicon.svg", alt: "", width: "48", height: "48" }),
      heading("Sign in to TessariDB S3"),
      el("p", { class: "muted" }, "Use the root access key this node was started with. The session lasts one hour."),
      key.row,
      secret.row,
      submit,
      said
    );
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      submit.disabled = true;
      const answer = await call("POST", "/session", ignored, {
        access_key_id: key.input.value.trim(),
        secret_access_key: secret.input.value
      });
      secret.input.value = "";
      submit.disabled = false;
      if (answer.ok) {
        key.input.value = "";
        done();
        return;
      }
      const failure = answer.status === 401 ? { ...answer, message: "That access key and secret do not match this node's root credential." } : answer;
      fill(said, refusal(failure));
      secret.input.focus();
    });
    fill(main2, el("div", { class: "gate" }, form));
    key.input.focus();
  }

  // src/meter.ts
  //! How full a drive is: a bar on a zero baseline whose length is the used share,
  //! with the figures beside it in text, so the bar's colour is never the only
  //! signal. The thresholds turn it amber, then red, before the drive is full.
  var WARN = 0.8;
  var CRITICAL = 0.95;
  var used = (drive2) => Math.max(drive2.capacity - drive2.free, 0);
  function meter(drive2, label) {
    const taken = used(drive2);
    const ratio = drive2.capacity === 0 ? 0 : taken / drive2.capacity;
    const level = ratio >= CRITICAL ? "bad" : ratio >= WARN ? "warn" : "ok";
    const bar = el("div", {
      class: `meter ${level}`,
      role: "meter",
      "aria-label": label,
      "aria-valuemin": "0",
      "aria-valuemax": String(drive2.capacity),
      "aria-valuenow": String(taken),
      "aria-valuetext": `${size(taken)} of ${size(drive2.capacity)} used, ${share(taken, drive2.capacity)}`
    });
    bar.style.setProperty("--fill", ratio.toFixed(4));
    return el(
      "div",
      { class: "gauge" },
      bar,
      el("span", { class: "gauge-text" }, `${size(taken)} of ${size(drive2.capacity)}`, el("span", { class: "muted" }, ` · ${share(taken, drive2.capacity)}`))
    );
  }

  // src/status.ts
  //! The overview: is this node and its cluster healthy, how much is stored and
  //! how full the drives are, and is anything waiting to be healed. Each tile
  //! links to where the operator acts on it.
  var TITLE4 = "Overview";
  function tile(glyph, label, value, note, href) {
    const parts = [el("span", { class: "label" }, icon(glyph), label), el("span", { class: "value" }, value), el("span", { class: "note" }, note)];
    return href === void 0 ? el("div", { class: "tile" }, ...parts) : el("a", { class: "tile", href }, ...parts);
  }
  function healing(heal) {
    if (heal === null) {
      return tile("pulse", "Healing", "—", "Applies to cluster members; this node stores objects on its own.");
    }
    if (heal.listed === 0) {
      return tile("pulse", "Healing", el("span", { class: "chip ok" }, "Healthy"), "No objects are waiting to be healed.");
    }
    const counted = heal.more ? `> ${heal.listed.toLocaleString()}` : heal.listed.toLocaleString();
    return tile("pulse", "Healing", counted, el("span", {}, el("span", { class: "chip warn" }, "Waiting"), " Members heal these objects in the background."));
  }
  function state(member2) {
    return member2.answering ? el("span", { class: "chip ok" }, "Answering") : el("span", { class: "chip bad" }, "Not answering");
  }
  function membersTile(members) {
    if (members === null) {
      return tile("members", "Members", "—", "No cluster");
    }
    const answering = members.filter((member2) => member2.answering).length;
    return answering === members.length ? tile("members", "Members", String(members.length), "Registered and answering") : tile("members", "Members", String(members.length), el("span", { class: "chip bad" }, `${answering} of ${members.length} answering`));
  }
  function storedTile(usage) {
    if (!usage.ok) {
      return tile("buckets", "Stored", "—", "The usage figures could not be read. Reload to try again.");
    }
    const measured = usage.value;
    if (measured.taken === null) {
      return tile("buckets", "Stored", "—", "Not measured yet. A node measures every minute after it starts.");
    }
    const objects2 = measured.objects === 1 ? "1 object" : `${amount(measured.objects)} objects`;
    return tile("buckets", "Stored", size(measured.bytes), `${objects2} · measured ${moment(measured.taken)}`);
  }
  function drives(node) {
    if (node.members === null) {
      return node.drive === null ? [] : [node.drive];
    }
    return node.members.flatMap((member2) => member2.drive === null ? [] : [member2.drive]);
  }
  function diskTile(node) {
    const read = drives(node);
    if (read.length === 0) {
      return tile("disk", "Disk", "—", node.members === null ? "This node stores no data on its own drive." : "No member reported its drive.");
    }
    const total = read.reduce(
      (sum, drive2) => ({ capacity: sum.capacity + drive2.capacity, free: sum.free + drive2.free, available: sum.available + drive2.available }),
      { capacity: 0, free: 0, available: 0 }
    );
    const scope = node.members === null ? "This node's data drive" : `Raw space on ${read.length} of ${node.members.length} members`;
    return tile(
      "disk",
      "Disk",
      el("span", {}, share(used(total), total.capacity), el("span", { class: "muted" }, " used")),
      el("span", { class: "gauge" }, meter(total, "Disk space used"), el("span", {}, `${scope} · ${size(total.free)} free`))
    );
  }
  async function status(screen) {
    loading(screen, TITLE4, "the node's status");
    const [answer, usage] = await Promise.all([call("GET", "/status", readStatus), call("GET", "/usage", readUsage)]);
    if (!answer.ok) {
      failed(screen, TITLE4, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const node = answer.value;
    const members = node.members === null ? null : node.members.length === 0 ? empty("No members are registered yet.") : table(
      "Cluster members",
      ["Member", "State", "Disk", "Internal address"],
      node.members.map(
        (member2) => row(mono(member2.node), state(member2), member2.drive === null ? "—" : meter(member2.drive, `Disk space used on ${member2.node}`), mono(member2.endpoint))
      )
    );
    fill(
      screen.main,
      head(TITLE4, el("span", {}, "Region ", mono(node.region), " · version ", mono(node.version))),
      el(
        "div",
        { class: "tiles" },
        tile("node", "Node", node.node === null ? "Single" : mono(node.node), node.node === null ? "Not a cluster member" : "This node's name in the cluster"),
        membersTile(node.members),
        tile("layers", "Erasure code", node.erasure === null ? "None" : mono(node.erasure), node.erasure === null ? "Whole objects on one node" : "Data + parity shards per object"),
        healing(node.heal_backlog),
        storedTile(usage),
        diskTile(node),
        tile("buckets", "Buckets", "Browse", "Find a bucket or an object", format({ kind: "buckets" })),
        tile("record", "Action record", "Review", "Every console change and download", format({ kind: "actions", before: null }))
      ),
      members === null ? null : el("section", { class: "card flush" }, el("h2", {}, "Members"), members)
    );
  }

  // src/theme.ts
  //! Light or dark. Until the operator chooses, the page follows the system's
  //! preference; a choice is remembered in this browser only (it is a viewer's
  //! convenience, not console state), and a browser that refuses storage simply
  //! forgets it.
  var KEY = "tessaridb-s3-console-theme";
  var darkQuery = matchMedia("(prefers-color-scheme: dark)");
  function remembered() {
    try {
      const value = localStorage.getItem(KEY);
      return value === "light" || value === "dark" ? value : null;
    } catch {
      return null;
    }
  }
  function remember(theme) {
    try {
      localStorage.setItem(KEY, theme);
    } catch {
    }
  }
  var current = () => remembered() ?? (darkQuery.matches ? "dark" : "light");
  function themes(button) {
    const show = () => {
      const theme = current();
      if (remembered() === null) {
        document.documentElement.removeAttribute("data-theme");
      } else {
        document.documentElement.setAttribute("data-theme", theme);
      }
      button.replaceChildren(icon(theme === "dark" ? "sun" : "moon"), theme === "dark" ? "Light theme" : "Dark theme");
    };
    button.addEventListener("click", () => {
      remember(current() === "dark" ? "light" : "dark");
      show();
    });
    darkQuery.addEventListener("change", show);
    show();
  }

  // src/console.ts
  //! The console's entry point: one view at a time, chosen by the URL's hash, so
  //! a reload or a shared link lands on the same view. A view that answers 401
  //! hands over to signing in, and signing in draws the same view again.
  var main = at("view");
  var signOut = at("sign-out");
  var generation = 0;
  function draw(route, screen) {
    switch (route.kind) {
      case "status":
        return status(screen);
      case "buckets":
        return buckets(screen);
      case "objects":
        return objects(screen, route.bucket, route.prefix, route.cursor);
      case "object":
        return object(screen, route.bucket, route.key);
      case "actions":
        return actions(screen, route.before);
    }
  }
  function mark(route) {
    const section = route.kind === "objects" || route.kind === "object" ? "buckets" : route.kind;
    for (const link of all("[data-section]")) {
      if (link.dataset["section"] === section) {
        link.setAttribute("aria-current", "page");
      } else {
        link.removeAttribute("aria-current");
      }
    }
  }
  function showSignIn() {
    generation += 1;
    signOut.hidden = true;
    document.body.classList.add("signed-out");
    signIn(main, () => {
      signOut.hidden = false;
      document.body.classList.remove("signed-out");
      announce("Signed in.");
      void render();
    });
  }
  async function render() {
    generation += 1;
    const mine = generation;
    const route = parse(location.hash);
    mark(route);
    const screen = {
      main,
      live: () => mine === generation,
      signIn: showSignIn,
      redraw: () => void render()
    };
    await draw(route, screen);
    if (screen.live()) {
      main.querySelector(".view-title")?.focus({ preventScroll: true });
    }
  }
  signOut.addEventListener("click", async () => {
    await call("DELETE", "/session", ignored);
    announce("Signed out.");
    showSignIn();
  });
  var SECTION_ICONS = { status: "overview", buckets: "buckets", actions: "record" };
  for (const link of all("[data-section]")) {
    const name = SECTION_ICONS[link.dataset["section"] ?? ""];
    if (name !== void 0) {
      link.prepend(icon(name));
    }
  }
  signOut.prepend(icon("sign-out"));
  themes(at("theme"));
  at("skip").addEventListener("click", (event) => {
    event.preventDefault();
    main.focus();
  });
  window.addEventListener("hashchange", () => void render());
  void render();
})();
