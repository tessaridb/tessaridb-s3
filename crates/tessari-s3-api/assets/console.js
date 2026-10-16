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
    if (!record(v) || !text(v["node"]) || !text(v["endpoint"]) || !textOrNull(v["zone"]) || typeof v["answering"] !== "boolean") {
      return null;
    }
    const space2 = drive(v["drive"]);
    return space2 === void 0 ? null : { node: v["node"], endpoint: v["endpoint"], zone: v["zone"], answering: v["answering"], drive: space2 };
  }
  function readStatus(v) {
    if (!record(v) || !text(v["version"]) || !textOrNull(v["node"]) || !text(v["region"]) || !textOrNull(v["erasure"])) {
      return null;
    }
    const members = v["members"] === null ? null : list(v["members"], member);
    const backlog = v["heal_backlog"];
    const heal = backlog === null ? null : record(backlog) && count(backlog["listed"]) && typeof backlog["more"] === "boolean" && count(backlog["at_risk"]) ? { listed: backlog["listed"], more: backlog["more"], at_risk: backlog["at_risk"] } : void 0;
    const sent = v["replication"];
    const replication = sent === null ? null : record(sent) && count(sent["backlog"]) && count(sent["in_flight"]) && count(sent["failed"]) ? { backlog: sent["backlog"], in_flight: sent["in_flight"], failed: sent["failed"] } : void 0;
    const space2 = drive(v["drive"]);
    if (v["members"] !== null && members === null || heal === void 0 || replication === void 0 || space2 === void 0) {
      return null;
    }
    return { version: v["version"], node: v["node"], region: v["region"], erasure: v["erasure"], members, heal_backlog: heal, replication, drive: space2 };
  }
  var bucketUsage = (v) => record(v) && text(v["bucket"]) && count(v["objects"]) && count(v["bytes"]) && count(v["raw_bytes"]) ? { bucket: v["bucket"], objects: v["objects"], bytes: v["bytes"], raw_bytes: v["raw_bytes"] } : null;
  function readUsage(v) {
    if (!record(v) || !textOrNull(v["taken"]) || !flag(v["stale"]) || !count(v["objects"]) || !count(v["bytes"]) || !count(v["raw_bytes"])) {
      return null;
    }
    const buckets2 = list(v["buckets"], bucketUsage);
    return buckets2 === null ? null : { taken: v["taken"], stale: v["stale"], buckets: buckets2, objects: v["objects"], bytes: v["bytes"], raw_bytes: v["raw_bytes"] };
  }
  var acknowledgement = (v) => v === "majority" || v === "region";
  function bucketEncryption(v) {
    if (v === null) {
      return null;
    }
    if (!record(v) || typeof v["customer_keys"] !== "boolean") {
      return void 0;
    }
    const customerKeys = v["customer_keys"];
    if (v["algorithm"] === "AES256" && v["key"] === null) {
      return { algorithm: "AES256", customerKeys };
    }
    return v["algorithm"] === "aws:kms" && text(v["key"]) ? { algorithm: "aws:kms", key: v["key"], customerKeys } : void 0;
  }
  function bucket(v) {
    if (!record(v)) {
      return null;
    }
    const encryption = bucketEncryption(v["encryption"]);
    return text(v["name"]) && text(v["created"]) && text(v["region"]) && countOrNull(v["max_bytes"]) && countOrNull(v["max_objects"]) && acknowledgement(v["acknowledgement"]) && encryption !== void 0 ? {
      name: v["name"],
      created: v["created"],
      region: v["region"],
      maxBytes: v["max_bytes"],
      maxObjects: v["max_objects"],
      acknowledgement: v["acknowledgement"],
      encryption
    } : null;
  }
  function readBuckets(v) {
    return record(v) ? list(v["buckets"], bucket) : null;
  }
  var objectRow = (v) => record(v) && text(v["key"]) && count(v["size"]) && text(v["etag"]) && text(v["modified"]) && textOrNull(v["content_type"]) ? { key: v["key"], size: v["size"], etag: v["etag"], modified: v["modified"], contentType: v["content_type"] } : null;
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
    const tags = strings(v["tags"]);
    const kept = v["retention"];
    const retention = kept === null ? null : readRetention(kept);
    if (headers === null || metadata === null || checksums === null || tags === null || retention === void 0 || !flag(v["legal_hold"])) {
      return null;
    }
    return {
      key: v["key"],
      size: v["size"],
      etag: v["etag"],
      modified: v["modified"],
      headers,
      metadata,
      checksums,
      parts: v["parts"],
      tags,
      retention,
      legalHold: v["legal_hold"]
    };
  }
  var lockMode = (value) => value === "GOVERNANCE" || value === "COMPLIANCE";
  var readRetention = (v) => record(v) && lockMode(v["mode"]) && text(v["until"]) ? { mode: v["mode"], until: v["until"] } : void 0;
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
  var flag = (value) => typeof value === "boolean";
  var role = (value) => value === "space_admin" || value === "member";
  var user = (v) => record(v) && text(v["name"]) && text(v["space"]) && role(v["role"]) && flag(v["create_buckets"]) && flag(v["operator"]) && flag(v["cluster_viewer"]) && flag(v["disabled"]) && text(v["created"]) ? {
    name: v["name"],
    space: v["space"],
    role: v["role"],
    create_buckets: v["create_buckets"],
    operator: v["operator"],
    cluster_viewer: v["cluster_viewer"],
    disabled: v["disabled"],
    created: v["created"]
  } : null;
  function readUsers(v) {
    return record(v) ? list(v["users"], user) : null;
  }
  var space = (v) => record(v) && text(v["name"]) && text(v["created"]) ? { name: v["name"], created: v["created"] } : null;
  function readSpaces(v) {
    return record(v) ? list(v["spaces"], space) : null;
  }
  function readIssued(v) {
    return record(v) && text(v["access_key_id"]) && text(v["secret_access_key"]) ? { access_key_id: v["access_key_id"], secret_access_key: v["secret_access_key"] } : null;
  }
  function readUploadKey(v) {
    const key = readIssued(v);
    return key !== null && record(v) && text(v["expires"]) ? { ...key, expires: v["expires"] } : null;
  }
  function readCapabilities(v) {
    return record(v) && text(v["access_key_id"]) && flag(v["operate"]) && flag(v["administer"]) && flag(v["view_cluster"]) && flag(v["issue_upload_keys"]) ? {
      access_key_id: v["access_key_id"],
      operate: v["operate"],
      administer: v["administer"],
      view_cluster: v["view_cluster"],
      issue_upload_keys: v["issue_upload_keys"]
    } : null;
  }
  var names = (value) => list(value, (entry) => text(entry) ? entry : null);
  function layout(v) {
    if (v === null) {
      return null;
    }
    const placed = (entry) => record(entry) && text(entry["node"]) && textOrNull(entry["zone"]) ? { node: entry["node"], zone: entry["zone"] } : null;
    const sets2 = record(v) && count(v["version"]) ? list(v["sets"], (set) => list(set, placed)) : null;
    return record(v) && count(v["version"]) && sets2 !== null ? { version: v["version"], sets: sets2 } : void 0;
  }
  function readCluster(v) {
    if (!record(v) || !textOrNull(v["node"]) || !textOrNull(v["erasure"]) || !record(v["metadata"])) {
      return null;
    }
    const members = v["members"] === null ? null : list(v["members"], member);
    const placed = layout(v["layout"]);
    const backlog = v["heal_backlog"];
    const addresses = names(v["metadata"]["addresses"]);
    if (v["members"] !== null && members === null || placed === void 0 || addresses === null) {
      return null;
    }
    if (!record(backlog) || !count(backlog["listed"]) || !flag(backlog["more"]) || !count(backlog["at_risk"])) {
      return null;
    }
    const moving2 = v["moving"];
    if (!record(moving2) || !count(moving2["listed"]) || !flag(moving2["more"])) {
      return null;
    }
    return {
      node: v["node"],
      erasure: v["erasure"],
      members,
      layout: placed,
      heal_backlog: { listed: backlog["listed"], more: backlog["more"], at_risk: backlog["at_risk"] },
      moving: { listed: moving2["listed"], more: moving2["more"] },
      metadata: { addresses }
    };
  }
  function readProblem(v) {
    return record(v) && text(v["code"]) && text(v["message"]) ? { code: v["code"], message: v["message"] } : null;
  }
  var ignored = (_v) => true;
  var renditionQueue = (value) => value === "images" || value === "videos";
  var failedRendition = (v) => record(v) && renditionQueue(v["queue"]) && text(v["bucket"]) && text(v["key"]) && textOrNull(v["version_id"]) && text(v["output"]) && count(v["attempts"]) && textOrNull(v["error"]) ? {
    queue: v["queue"],
    bucket: v["bucket"],
    key: v["key"],
    version_id: v["version_id"],
    output: v["output"],
    attempts: v["attempts"],
    error: v["error"]
  } : null;
  function readFailedRenditions(v) {
    if (!record(v) || !flag(v["more"])) {
      return null;
    }
    const failed2 = list(v["failed"], failedRendition);
    return failed2 === null ? null : { failed: failed2, more: v["more"] };
  }

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
    const made2 = document.createElement(tag);
    for (const [name, value] of Object.entries(attributes)) {
      made2.setAttribute(name, value);
    }
    for (const child of children) {
      if (child !== null && child !== false) {
        made2.append(child);
      }
    }
    return made2;
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
  function segmented(id, label, options) {
    const inputs = options.map((option, at2) => {
      const input = el("input", { type: "radio", name: id, id: `${id}-${at2}`, value: option });
      if (at2 === 0) {
        input.checked = true;
      }
      return input;
    });
    const group = el(
      "div",
      { class: "segmented", role: "radiogroup", "aria-label": label },
      ...inputs.flatMap((input) => [input, el("label", { for: input.id }, input.value)])
    );
    const chosen = () => inputs.find((input) => input.checked)?.value ?? "";
    const choose = (value) => {
      for (const input of inputs) {
        input.checked = input.value === value;
      }
    };
    return { group, chosen, choose };
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
  var QUOTA_UNITS = { MiB: 1024 ** 2, GiB: 1024 ** 3, TiB: 1024 ** 4 };
  function bytesOf(amount2, unit) {
    const written = amount2.trim();
    if (written === "") {
      return null;
    }
    if (!/^\d+(\.\d+)?$/.test(written)) {
      return void 0;
    }
    const value = Number(written);
    const bytes = Math.round(value * QUOTA_UNITS[unit]);
    return Number.isSafeInteger(bytes) && (bytes > 0 || value === 0) ? bytes : void 0;
  }

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
      case "users":
        return "#/users";
      case "spaces":
        return "#/spaces";
      case "cluster":
        return "#/cluster";
      case "renditions":
        return "#/renditions";
      case "objects":
        return `#/b/${encodeURIComponent(route.bucket)}${query([
          ["prefix", route.prefix === "" ? null : route.prefix],
          ["filter", route.filter === "" ? null : route.filter],
          ["cursor", route.cursor],
          ["key", route.key]
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
      if (path2 === "/users") {
        return { kind: "users" };
      }
      if (path2 === "/spaces") {
        return { kind: "spaces" };
      }
      if (path2 === "/cluster") {
        return { kind: "cluster" };
      }
      if (path2 === "/renditions") {
        return { kind: "renditions" };
      }
      if (path2 === "/actions") {
        return { kind: "actions", before: position(params.get("before")) };
      }
      if (path2.startsWith("/b/")) {
        const bucket2 = decodeURIComponent(path2.slice(3));
        return bucket2 === "" ? { kind: "buckets" } : { kind: "objects", bucket: bucket2, prefix: params.get("prefix") ?? "", cursor: params.get("cursor"), key: params.get("key") || null, filter: params.get("filter") ?? "" };
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
      case "forbidden":
        return "Your key does not allow this. Ask an operator or your space's administrator.";
      case "rate_limit":
        return "Wait a minute, then try again.";
      case "unavailable":
      case "network":
        return "Try again in a moment.";
      case "precondition_failed":
        return "Reload it and check it before acting.";
      case "reauthentication_required":
        return "Sign out, sign in again, then repeat this: changes like this need a sign-in from the last five minutes.";
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
    const retry = failure.status === 403 ? null : el("button", { type: "button" }, "Try again");
    retry?.addEventListener("click", screen.redraw);
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
  //! sit in. Each is hidden from screen readers: it sits beside a text label, or
  //! inside an icon button whose accessible name says the action and its target.
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
    text: ["M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z", "M14 3v5h5", "M9 13h6", "M9 17h6"],
    image: [box(3, 4, 18, 16, 2), ring(8.5, 9.5, 1.5), "M21 15.5l-5-5L5 20"],
    video: [box(3, 5, 18, 14, 2), "M10 9l5 3-5 3z"],
    audio: ["M9 18V6l10-2v12", ring(6.5, 18, 2.5), ring(16.5, 16, 2.5)],
    archive: [box(3, 4, 18, 5, 1.5), "M5 9v9a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V9", "M10 13h4"],
    download: ["M12 4v11", "M7.5 10.5L12 15l4.5-4.5", "M5 20h14"],
    trash: ["M4 7h16", "M9.5 7V4.5h5V7", "M6.5 7l1 13h9l1-13", "M10 11v5.5", "M14 11v5.5"],
    plus: ["M12 5v14", "M5 12h14"],
    close: ["M6 6l12 12", "M18 6L6 18"],
    info: [ring(12, 12, 9), "M12 11v5", "M12 7.5h.01"],
    copy: [box(8, 8, 13, 13, 2), "M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"],
    users: [ring(9, 8, 3.5), "M3 20c0-3.3 2.7-6 6-6s6 2.7 6 6", "M16 4.5a3.5 3.5 0 0 1 0 7", "M18 14c2 .6 3 2.8 3 6"],
    chevron: ["M9.5 6l6 6-6 6"],
    node: [box(3, 4, 18, 7, 2), box(3, 13, 18, 7, 2), "M7 7.5h.01", "M7 16.5h.01"],
    members: [ring(6, 7, 2.5), ring(18, 7, 2.5), ring(12, 18, 2.5), "M8.5 7h7", "M7.3 9.2l3.4 6.6", "M16.7 9.2l-3.4 6.6"],
    layers: ["M12 3l9 5-9 5-9-5z", "M3 13l9 5 9-5"],
    pulse: ["M3 12h4l3-7 4 14 3-7h4"],
    back: ["M14.5 6l-6 6 6 6"],
    disk: [box(3, 5, 18, 14, 2.5), "M3 13h18", "M16.5 16h.01"],
    key: [ring(7.5, 15.5, 4), "M10.5 12.5L20 3", "M16 7l3 3", "M14 9l2 2"],
    keys: [ring(6.5, 16.5, 3.5), "M9 14l7-7", "M13.5 9.5l2 2", "M14 5h7", "M17 10h4", "M15 15h6"],
    ban: [ring(12, 12, 9), "M5.6 5.6l12.8 12.8"],
    check: [ring(12, 12, 9), "M8 12.5l2.5 2.5L16 9.5"],
    shield: ["M12 3l8 3v6c0 4.4-3.4 8.2-8 9-4.6-.8-8-4.6-8-9V6z", "M9 12l2 2 4-4"],
    sliders: ["M4 6h9", "M17 6h3", ring(15, 6, 2), "M4 12h3", "M11 12h9", ring(9, 12, 2), "M4 18h11", "M19 18h1", ring(17, 18, 2)],
    gauge: ["M4 18a8 8 0 1 1 16 0", "M12 18l4-6", "M12 18h.01"],
    upload: ["M12 20V9", "M7.5 13.5L12 9l4.5 4.5", "M5 4h14"]
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

  // src/icon-action.ts
  //! A row's action as an icon button with a tooltip. The accessible name says
  //! the action and the row it acts on ("Revoke key TS…"); the tooltip says the
  //! action alone, because the row is beside it. The tooltip is one shared
  //! element placed over the page, so a scrolling table never clips it; it opens
  //! after a short pause under the pointer — at once when another tip was just
  //! open, or when the button takes keyboard focus — and Escape closes it.
  var OPEN_DELAY_MS = 500;
  var INSTANT_WINDOW_MS = 400;
  function iconAction(name, tip, label, tone = "quiet") {
    return el("button", { type: "button", class: `icon-action ${tone}`, "aria-label": label, "data-tip": tip }, icon(name));
  }
  function installTips() {
    const tip = el("div", { class: "tip", role: "tooltip", "aria-hidden": "true" });
    document.body.append(tip);
    let anchor = null;
    let timer;
    let closedAt = Number.NEGATIVE_INFINITY;
    const place = (target2) => {
      const box2 = target2.getBoundingClientRect();
      const own = tip.getBoundingClientRect();
      const above = box2.top - own.height - 6;
      const below = above < 4;
      const left = Math.min(Math.max(4, box2.left + box2.width / 2 - own.width / 2), window.innerWidth - own.width - 4);
      tip.style.left = `${left}px`;
      tip.style.top = `${below ? box2.bottom + 6 : above}px`;
      tip.dataset["side"] = below ? "below" : "above";
    };
    const show = (target2, instant) => {
      anchor = target2;
      tip.textContent = target2.dataset["tip"] ?? "";
      tip.toggleAttribute("data-instant", instant);
      tip.dataset["open"] = "";
      place(target2);
    };
    const hide = () => {
      window.clearTimeout(timer);
      if (anchor !== null) {
        closedAt = performance.now();
      }
      anchor = null;
      delete tip.dataset["open"];
    };
    const target = (event) => event.target instanceof Element ? event.target.closest("[data-tip]") : null;
    document.addEventListener("pointerover", (event) => {
      const over = target(event);
      if (over === null || over === anchor) {
        return;
      }
      hide();
      const instant = performance.now() - closedAt < INSTANT_WINDOW_MS;
      timer = window.setTimeout(() => show(over, instant), instant ? 0 : OPEN_DELAY_MS);
    });
    document.addEventListener("pointerout", (event) => {
      if (target(event) !== null) {
        hide();
      }
    });
    document.addEventListener("focusin", (event) => {
      const focused = target(event);
      if (focused !== null && focused.matches(":focus-visible")) {
        hide();
        show(focused, true);
      }
    });
    document.addEventListener("focusout", hide);
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        hide();
      }
    });
    window.addEventListener("scroll", hide, { capture: true, passive: true });
  }

  // src/bucket-acknowledgement.ts
  //! Who acknowledges a bucket's object writes: the majority of the nodes keeping
  //! its region's metadata (the default), or that region's majority alone. The
  //! second is faster when the metadata is also kept in other regions, and it is
  //! the one that can lose writes: an acknowledged write the region's nodes held
  //! is lost if the metadata's leadership fails over to a node that did not
  //! collect it. The server proves the region level before setting it and shows
  //! the store's own refusal when there is no region to count. Operators only;
  //! the reason goes into the action record.
  function acknowledgementForm(screen, bucket2, close) {
    const id = `ack-${bucket2.name}`;
    const choice = (value, label, note) => {
      const input = el("input", { type: "radio", name: `${id}-level`, id: `${id}-${value}`, value });
      if (bucket2.acknowledgement === value) {
        input.setAttribute("checked", "");
      }
      return el("div", { class: "choice" }, input, el("label", { for: `${id}-${value}` }, el("strong", {}, label), " — ", note));
    };
    const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
    const save = el("button", { type: "submit", class: "primary" }, "Set");
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
      el("p", { id: `${id}-title` }, el("strong", {}, "Write acknowledgement: "), mono(bucket2.name)),
      choice("majority", "Majority", "a write is acknowledged once most of the nodes keeping this region's metadata hold it; it survives a failover."),
      choice("region", "Region majority", "acknowledged once most of this region's nodes hold it — faster, and a failover to a node outside the region can lose writes acknowledged this way."),
      reason.row,
      el("div", { class: "actions" }, save, cancel),
      said
    );
    cancel.addEventListener("click", close);
    form.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        close();
      }
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const picked = form.querySelector(`input[name="${id}-level"]:checked`);
      if (picked === null) {
        fill(said, el("p", { class: "message bad", role: "alert" }, "Choose one."));
        return;
      }
      const answer = await call("PUT", `/buckets/${encodeURIComponent(bucket2.name)}/acknowledgement`, ignored, {
        level: picked.value,
        reason: reason.input.value.trim()
      });
      if (answer.ok) {
        announce(`Set the write acknowledgement of ${bucket2.name}.`);
        screen.redraw();
        return;
      }
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      fill(said, refusal(answer));
    });
    return form;
  }

  // src/bucket-encryption.ts
  //! A bucket's default encryption: how a write that asks for no encryption is
  //! sealed — under the server's key, under a key an operator created by name, or
  //! not at all — and, beside a default, whether a write may bring its own key
  //! (SSE-C, blocked otherwise). The server sets it only while this node holds its
  //! keys and, for a named key, that key; its refusal is shown as it came. Who may
  //! configure the bucket sets it; the reason goes into the action record.
  var KEY_NAME = /^[a-z0-9-]{1,64}$/;
  function encryptionChip(bucket2) {
    const sealing = bucket2.encryption;
    if (sealing === null) {
      return null;
    }
    const named = sealing.algorithm === "AES256" ? "AES256" : `aws:kms · ${sealing.key}`;
    return el("span", { class: "chip ok", title: "Default encryption" }, sealing.customerKeys ? `${named} · SSE-C allowed` : named);
  }
  function encryptionForm(screen, bucket2, close) {
    const id = `sse-${bucket2.name}`;
    const current2 = bucket2.encryption?.algorithm ?? "none";
    const choice = (value, label, note) => {
      const input = el("input", { type: "radio", name: `${id}-algorithm`, id: `${id}-${value}`, value });
      if (current2 === value) {
        input.setAttribute("checked", "");
      }
      return el("div", { class: "choice" }, input, el("label", { for: `${id}-${value}` }, el("strong", {}, label), " — ", note));
    };
    const named = bucket2.encryption?.algorithm === "aws:kms" ? bucket2.encryption.key : "";
    const allow = el("input", { type: "checkbox", id: `${id}-customer-keys` });
    if (bucket2.encryption?.customerKeys === true) {
      allow.setAttribute("checked", "");
    }
    const customerKeys = el(
      "div",
      { class: "choice" },
      allow,
      el("label", { for: `${id}-customer-keys` }, el("strong", {}, "Allow customer keys"), " — a write may bring its own key (SSE-C) over TLS; blocked otherwise.")
    );
    const key = field(`${id}-key`, "Key name", { maxlength: "64", value: named, autocomplete: "off" }, "For aws:kms: a key created on the Encryption page.");
    const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
    const save = el("button", { type: "submit", class: "primary" }, "Set");
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
      el("p", { id: `${id}-title` }, el("strong", {}, "Default encryption: "), mono(bucket2.name)),
      choice("none", "None", "a write is sealed only when it asks to be."),
      choice("AES256", "Server key", "every write that asks for nothing is sealed under the server's key (SSE-S3)."),
      choice("aws:kms", "Named key", "every write that asks for nothing is sealed under the key named below (SSE-KMS)."),
      key.row,
      customerKeys,
      reason.row,
      el("div", { class: "actions" }, save, cancel),
      said
    );
    cancel.addEventListener("click", close);
    form.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        close();
      }
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const picked = form.querySelector(`input[name="${id}-algorithm"]:checked`);
      if (picked === null) {
        fill(said, el("p", { class: "message bad", role: "alert" }, "Choose one."));
        return;
      }
      const name = key.input.value.trim();
      if (picked.value === "aws:kms" && !KEY_NAME.test(name)) {
        fill(said, el("p", { class: "message bad", role: "alert" }, "A key name is 1-64 lowercase letters, digits and hyphens."));
        key.input.focus();
        return;
      }
      const customer = allow.checked;
      if (customer && picked.value === "none") {
        fill(said, el("p", { class: "message bad", role: "alert" }, "Customer keys are allowed only beside a default."));
        return;
      }
      const why = reason.input.value.trim();
      const body = picked.value === "aws:kms" ? { algorithm: picked.value, key: name, customer_keys: customer, reason: why } : { algorithm: picked.value, customer_keys: customer, reason: why };
      const answer = await call("PUT", `/buckets/${encodeURIComponent(bucket2.name)}/encryption`, ignored, body);
      if (answer.ok) {
        announce(`Set the default encryption of ${bucket2.name}.`);
        screen.redraw();
        return;
      }
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      fill(said, refusal(answer));
    });
    return form;
  }

  // src/bucket-quota.ts
  //! A bucket's quota: the limits as the row shows them, and the form an operator
  //! sets them with, inline under the row. Both limits are sent every time, so a
  //! repeated save changes nothing; an empty field is no limit. The reason is
  //! required and goes into the action record.
  var isUnit = (value) => Object.hasOwn(QUOTA_UNITS, value);
  function limits(bucket2) {
    const parts = [
      bucket2.maxBytes === null ? null : size(bucket2.maxBytes),
      bucket2.maxObjects === null ? null : `${amount(bucket2.maxObjects)} objects`
    ].filter((part) => part !== null);
    return parts.length === 0 ? "—" : parts.join(" · ");
  }
  function flag2(input, wrong) {
    if (wrong) {
      input.setAttribute("aria-invalid", "true");
    } else {
      input.removeAttribute("aria-invalid");
    }
  }
  function unitOf(bytes) {
    const units = ["TiB", "GiB", "MiB"];
    return units.find((unit) => bytes % QUOTA_UNITS[unit] === 0) ?? "MiB";
  }
  function quotaForm(screen, bucket2, close) {
    const id = `quota-${bucket2.name}`;
    const unit = segmented(`${id}-unit`, "Unit", Object.keys(QUOTA_UNITS));
    const bytes = field(`${id}-bytes`, "Size limit", { inputmode: "decimal", autocomplete: "off" }, "Empty: no size limit. Checked against the last measurement.");
    const objects2 = field(`${id}-objects`, "Object limit", { inputmode: "numeric", autocomplete: "off" }, "Empty: no object limit.");
    const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
    if (bucket2.maxBytes !== null) {
      unit.choose(unitOf(bucket2.maxBytes));
      bytes.input.value = String(bucket2.maxBytes / QUOTA_UNITS[unitOf(bucket2.maxBytes)]);
    }
    if (bucket2.maxObjects !== null) {
      objects2.input.value = String(bucket2.maxObjects);
    }
    bytes.input.after(unit.group);
    const save = el("button", { type: "submit", class: "primary" }, "Set quota");
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
      el("p", { id: `${id}-title` }, el("strong", {}, "Quota: "), mono(bucket2.name)),
      el("p", { class: "muted" }, "Writes that would pass a limit are refused. A bucket can pass it by what is written before the next measurement."),
      bytes.row,
      objects2.row,
      reason.row,
      el("div", { class: "actions" }, save, cancel),
      said
    );
    cancel.addEventListener("click", close);
    form.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        close();
      }
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const picked = unit.chosen();
      const maxBytes = isUnit(picked) ? bytesOf(bytes.input.value, picked) : void 0;
      const written = objects2.input.value.trim();
      const maxObjects = written === "" ? null : /^\d+$/.test(written) ? Number(written) : void 0;
      const objectsWrong = maxObjects === void 0 || maxObjects !== null && !Number.isSafeInteger(maxObjects);
      flag2(bytes.input, maxBytes === void 0);
      flag2(objects2.input, objectsWrong);
      if (maxBytes === void 0 || maxObjects === void 0 || objectsWrong) {
        fill(said, el("p", { class: "message bad", role: "alert" }, "Write each limit as a number, or leave it empty for none."));
        return;
      }
      const answer = await call("PUT", `/buckets/${encodeURIComponent(bucket2.name)}/quota`, ignored, {
        max_bytes: maxBytes,
        max_objects: maxObjects,
        reason: reason.input.value.trim()
      });
      if (answer.ok) {
        announce(`Set the quota of ${bucket2.name}.`);
        screen.redraw();
        return;
      }
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      fill(said, refusal(answer));
    });
    queueMicrotask(() => bytes.input.focus());
    return form;
  }

  // src/keys-model.ts
  //! What the console answers about a user's access keys: an issue (a new key with
  //! its secret, or the key the form's token already issued, without one) and a
  //! listing, which never carries a secret.
  function readIssue(v) {
    if (!record(v) || !text(v["access_key_id"])) {
      return null;
    }
    if (text(v["secret_access_key"])) {
      return { kind: "issued", key: { access_key_id: v["access_key_id"], secret_access_key: v["secret_access_key"] } };
    }
    return v["already_issued"] === true ? { kind: "already", access_key_id: v["access_key_id"] } : null;
  }
  function reach(v) {
    if (v === null) {
      return null;
    }
    return record(v) && text(v["bucket"]) && text(v["key"]) && count(v["expires"]) ? { bucket: v["bucket"], key: v["key"], expires: v["expires"] } : void 0;
  }
  function listed(v) {
    if (!record(v) || !text(v["access_key_id"]) || !text(v["created"]) || !flag(v["disabled"]) || !("upload" in v)) {
      return null;
    }
    const upload = reach(v["upload"]);
    return upload === void 0 ? null : { access_key_id: v["access_key_id"], created: v["created"], disabled: v["disabled"], upload };
  }
  function readKeys(v) {
    return record(v) ? list(v["keys"], listed) : null;
  }

  // src/row-region.ts
  //! The inline region a user row's action opens under the row, and how a failure
  //! inside one is settled.
  var COLUMNS = "6";
  function settle(screen, failure) {
    if (failure.status === 401) {
      screen.signIn();
      return null;
    }
    return failure;
  }
  function open(line, region) {
    const next2 = line.nextElementSibling;
    if (next2 instanceof HTMLTableRowElement && next2.classList.contains("asking")) {
      next2.remove();
    }
    const ask = el("tr", { class: "asking" }, el("td", { colspan: COLUMNS }, region));
    line.after(ask);
    return () => ask.remove();
  }

  // src/user-keys.ts
  //! A user's access keys, listed in a region under the user's row — never a
  //! secret, which the server does not hand back — each revocable. Revoking names
  //! the key, cannot be undone, and takes a reason; every node refuses the key
  //! within five seconds.
  function reach2(key) {
    if (key.upload === null) {
      return "Full";
    }
    const until = new Date(key.upload.expires * 1e3).toISOString();
    return el("span", {}, "Upload of ", mono(`${key.upload.bucket}/${key.upload.key}`), ` until ${moment(until)}`);
  }
  function keys(screen, user2, line, opener) {
    const region = el("div", { class: "keys", "aria-live": "polite" });
    const close = open(line, region);
    const back = () => {
      close();
      opener.focus();
    };
    const route = `/users/${encodeURIComponent(user2.name)}/keys`;
    const done5 = el("button", { type: "button", class: "quiet" }, "Close");
    done5.addEventListener("click", back);
    const revoke = (key, button) => {
      const ask = el("div", {});
      button.after(ask);
      fill(
        ask,
        confirmation(
          `revoke-${key.access_key_id}`,
          mono(key.access_key_id),
          `Every node refuses this key of ${user2.name} within five seconds. It cannot be restored; a new key has a new secret.`,
          "Revoke key",
          async (reason) => {
            const answer = await call("DELETE", `${route}/${encodeURIComponent(key.access_key_id)}`, ignored, { reason });
            if (!answer.ok) {
              return settle(screen, answer);
            }
            announce(`Revoked key ${key.access_key_id} of ${user2.name}.`);
            void show();
            return null;
          },
          () => {
            ask.remove();
            button.focus();
          }
        )
      );
    };
    const show = async () => {
      fill(region, el("p", { class: "muted" }, `Loading the keys of ${user2.name}…`));
      const answer = await call("GET", route, readKeys);
      if (!answer.ok) {
        const shown = settle(screen, answer);
        if (shown !== null) {
          fill(region, refusal(shown), done5);
        }
        return;
      }
      if (answer.value.length === 0) {
        fill(region, el("p", {}, `${user2.name} has no access keys.`), done5);
        return;
      }
      const rows = answer.value.map((key) => {
        const button = iconAction("trash", "Revoke…", `Revoke key ${key.access_key_id}`, "danger");
        button.addEventListener("click", () => revoke(key, button));
        return row(mono(key.access_key_id), moment(key.created), key.disabled ? "Disabled" : "Active", reach2(key), button);
      });
      fill(region, table(`Access keys of ${user2.name}`, ["Access key id", "Issued", "State", "Reach", ""], rows), done5);
    };
    void show();
  }

  // src/user-actions.ts
  //! A user row's actions, each opening an inline region under the row: issuing
  //! a key (its secret is shown here once and never again — the server keeps it
  //! sealed and will not hand it back), listing and revoking keys, disabling or
  //! enabling the user, and granting or removing access to one bucket. Every
  //! change takes a reason.
  function shownOnce(key, done5) {
    const copy = el("button", { type: "button" }, "Copy secret");
    const finish = el("button", { type: "button", class: "primary" }, "I have stored it");
    const said = el("p", { class: "hint", "aria-live": "polite" });
    copy.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(key.secret_access_key);
        said.textContent = "Copied.";
      } catch {
        said.textContent = "Copying is not allowed here; select the secret and copy it by hand.";
      }
    });
    finish.addEventListener("click", done5);
    queueMicrotask(() => copy.focus());
    return el(
      "div",
      { class: "once", role: "alert" },
      el("p", {}, el("strong", {}, "Store this secret now. "), "It is shown once; the server will not show it again."),
      el("dl", {}, el("dt", {}, "Access key id"), el("dd", {}, mono(key.access_key_id)), el("dt", {}, "Secret access key"), el("dd", {}, mono(key.secret_access_key))),
      el("div", { class: "actions" }, copy, finish),
      said
    );
  }
  function alreadyIssued(id, done5) {
    const finish = el("button", { type: "button", class: "primary" }, "Close");
    finish.addEventListener("click", done5);
    queueMicrotask(() => finish.focus());
    return el(
      "div",
      { class: "once", role: "status" },
      el("p", {}, "This form already issued key ", mono(id), ". Its secret was shown once and cannot be shown again; if it was not stored, revoke the key and issue a new one."),
      el("div", { class: "actions" }, finish)
    );
  }
  function issueKey(screen, user2, line, opener) {
    const region = el("div", {});
    const close = open(line, region);
    const back = () => {
      close();
      opener.focus();
    };
    const token = crypto.randomUUID();
    let secretShown = false;
    fill(
      region,
      confirmation(
        `key-${user2.name}`,
        mono(user2.name),
        "A new access key for this user. Its secret is shown once, right here.",
        "Issue key",
        async (reason) => {
          const answer = await call("POST", `/users/${encodeURIComponent(user2.name)}/keys`, readIssue, { reason, idempotency_key: token });
          if (!answer.ok) {
            return settle(screen, answer);
          }
          const issued = answer.value;
          if (issued.kind === "already") {
            if (secretShown) {
              return null;
            }
            announce(`This form already issued ${user2.name} a key.`);
            fill(region, alreadyIssued(issued.access_key_id, back));
            return null;
          }
          secretShown = true;
          announce(`Issued a key for ${user2.name}. Store its secret now.`);
          fill(region, shownOnce(issued.key, () => {
            fill(region);
            back();
          }));
          return null;
        },
        back
      )
    );
  }
  function setDisabled(screen, user2, line, opener) {
    const disable = !user2.disabled;
    const region = el("div", {});
    const close = open(line, region);
    const back = () => {
      close();
      opener.focus();
    };
    fill(
      region,
      confirmation(
        `state-${user2.name}`,
        mono(user2.name),
        disable ? "Every key of this user stops working within five seconds, on every node." : "The user's keys work again within five seconds.",
        disable ? "Disable user" : "Enable user",
        async (reason) => {
          const answer = await call("PUT", `/users/${encodeURIComponent(user2.name)}/disabled`, ignored, { disabled: disable, reason });
          if (!answer.ok) {
            return settle(screen, answer);
          }
          announce(`${disable ? "Disabled" : "Enabled"} ${user2.name}.`);
          screen.redraw();
          return null;
        },
        back
      )
    );
  }
  function grants(screen, user2, line, opener) {
    const id = `grant-${user2.name}`;
    const bucket2 = field(`${id}-bucket`, "Bucket", { spellcheck: "false", autocomplete: "off", required: "" }, `A bucket of space ${user2.space}.`);
    const read = el("input", { id: `${id}-read`, type: "checkbox", checked: "" });
    const write = el("input", { id: `${id}-write`, type: "checkbox" });
    const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
    const grant = el("button", { type: "submit", class: "primary" }, "Grant");
    const remove = el("button", { type: "button", class: "danger" }, "Remove grant");
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
      el("p", { id: `${id}-title` }, el("strong", {}, "Bucket access: "), mono(user2.name)),
      bucket2.row,
      el("fieldset", { class: "checks" }, el("legend", {}, "Access"), el("label", { class: "check", for: read.id }, read, "Read"), el("label", { class: "check", for: write.id }, write, "Write")),
      reason.row,
      el("div", { class: "actions" }, grant, remove, cancel),
      said
    );
    const close = open(line, form);
    const back = () => {
      close();
      opener.focus();
    };
    const route = () => `/users/${encodeURIComponent(user2.name)}/grants/${encodeURIComponent(bucket2.input.value.trim())}`;
    const done5 = (words) => {
      announce(words);
      back();
    };
    const show = (failure) => {
      const shown = settle(screen, failure);
      if (shown !== null) {
        fill(said, refusal(shown));
      }
    };
    cancel.addEventListener("click", back);
    form.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        back();
      }
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const answer = await call("PUT", route(), ignored, { read: read.checked, write: write.checked, reason: reason.input.value.trim() });
      if (answer.ok) {
        done5(`Granted ${user2.name} access to ${bucket2.input.value.trim()}.`);
      } else {
        show(answer);
      }
    });
    remove.addEventListener("click", async () => {
      const answer = await call("DELETE", route(), ignored, { reason: reason.input.value.trim() });
      if (answer.ok) {
        done5(`Removed ${user2.name}'s grant on ${bucket2.input.value.trim()}.`);
      } else {
        show(answer);
      }
    });
    queueMicrotask(() => bucket2.input.focus());
  }
  function rowActions(screen, user2, line) {
    const key = iconAction("key", "Issue key…", `Issue a key to ${user2.name}`);
    const list2 = iconAction("keys", "Keys…", `Keys of ${user2.name}`);
    const state2 = user2.disabled ? iconAction("check", "Enable…", `Enable ${user2.name}`) : iconAction("ban", "Disable…", `Disable ${user2.name}`);
    const access = iconAction("shield", "Bucket access…", `Bucket access of ${user2.name}`);
    key.addEventListener("click", () => issueKey(screen, user2, line, key));
    list2.addEventListener("click", () => keys(screen, user2, line, list2));
    state2.addEventListener("click", () => setDisabled(screen, user2, line, state2));
    access.addEventListener("click", () => grants(screen, user2, line, access));
    return [key, list2, state2, access];
  }

  // src/bucket-upload-key.ts
  //! Issuing a one-key upload credential for a bucket, inline under its row: the
  //! key it may upload, how long it lasts, and a reason that is recorded. The
  //! credential uploads that one key until it expires, and only while the issuer
  //! may still write the bucket. Its secret is shown once, here.
  var LIFETIMES = { "1 hour": 3600, "1 day": 86400, "7 days": 604800 };
  var isLifetime = (value) => Object.hasOwn(LIFETIMES, value);
  function uploadKeyForm(screen, bucket2, close) {
    const id = `upload-${bucket2.name}`;
    const key = field(`${id}-key`, "Object key", { autocomplete: "off", required: "" }, "The one key it may upload, exactly as written.");
    const lasts = segmented(`${id}-lasts`, "Lasts", Object.keys(LIFETIMES));
    const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
    key.input.after(lasts.group);
    const issue = el("button", { type: "submit", class: "primary" }, "Issue upload key");
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    const said = el("div", { "aria-live": "polite" });
    const region = el("div", {});
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
      el("p", { id: `${id}-title` }, el("strong", {}, "Upload key: "), mono(bucket2.name)),
      el("p", { class: "muted" }, "It uploads one key — a single PUT or a multipart upload — and nothing else, until it expires or you lose write access here."),
      key.row,
      reason.row,
      el("div", { class: "actions" }, issue, cancel),
      said
    );
    fill(region, form);
    cancel.addEventListener("click", close);
    form.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        close();
      }
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const wanted = key.input.value;
      const lifetime = lasts.chosen();
      if (wanted === "" || !isLifetime(lifetime)) {
        key.input.setAttribute("aria-invalid", "true");
        fill(said, el("p", { class: "message bad", role: "alert" }, "Name the key it may upload."));
        return;
      }
      key.input.removeAttribute("aria-invalid");
      const answer = await call("POST", `/buckets/${encodeURIComponent(bucket2.name)}/upload-keys`, readUploadKey, {
        key: wanted,
        expires_in: LIFETIMES[lifetime],
        reason: reason.input.value.trim()
      });
      if (answer.ok) {
        announce(`Issued an upload key for ${bucket2.name}/${wanted}. Store its secret now.`);
        fill(
          region,
          el("p", {}, "Uploads ", mono(`${bucket2.name}/${wanted}`), " until ", moment(answer.value.expires), "."),
          shownOnce(answer.value, close)
        );
        return;
      }
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      fill(said, refusal(answer));
    });
    queueMicrotask(() => key.input.focus());
    return region;
  }

  // src/media-model.ts
  //! A bucket's media rules as the console sends and answers them: which uploads
  //! (content types under a key prefix) get which renditions. An output is
  //! `kind/size/format`, each picked from the server's lists; the server checks a
  //! rule set as a whole, this page only reads the answer and shapes the form.
  var IMAGE_SIZES = ["150", "320", "640", "768", "1024", "1280", "1440", "1920", "2560", "3840"];
  var STILL_FORMATS = ["jpeg", "png", "webp", "avif"];
  var KINDS = {
    image: { label: "Image", sizes: IMAGE_SIZES, formats: STILL_FORMATS, from: "image" },
    video: { label: "Video", sizes: ["360p", "480p", "720p", "1080p", "1440p", "2160p"], formats: ["mp4", "webm"], from: "video" },
    poster: { label: "Poster frame", sizes: IMAGE_SIZES, formats: STILL_FORMATS, from: "video" }
  };
  var isKind = (value) => Object.hasOwn(KINDS, value);
  function outputOf(name) {
    const [kind, size2, format2, extra] = name.split("/");
    if (kind === void 0 || size2 === void 0 || format2 === void 0 || extra !== void 0 || !isKind(kind)) {
      return null;
    }
    const lists = KINDS[kind];
    return lists.sizes.includes(size2) && lists.formats.includes(format2) ? { kind, size: size2, format: format2 } : null;
  }
  var spell = (output) => `${output.kind}/${output.size}/${output.format}`;
  function rule(v) {
    if (!record(v) || !text(v["id"]) || !text(v["prefix"])) {
      return null;
    }
    const types = list(v["content_types"], (entry) => text(entry) ? entry : null);
    const outputs = list(v["outputs"], (entry) => text(entry) && outputOf(entry) !== null ? entry : null);
    return types === null || outputs === null ? null : { id: v["id"], content_types: types, prefix: v["prefix"], outputs };
  }
  function readMediaRules(v) {
    return record(v) ? list(v["rules"], rule) : null;
  }
  var typesOf = (written) => written.split(",").map((part) => part.trim()).filter((part) => part !== "");

  // src/bucket-media.ts
  //! A bucket's media rules, inline under its row: anyone who reads the bucket
  //! sees them, and whoever configures it changes them — the server decides which
  //! one the signed-in key is. The rules are sent whole, so a repeated save
  //! changes nothing; saving none turns media processing off.
  var made = 0;
  function outputRow(id, spelled, remove) {
    const fallback = { kind: "image", size: "1280", format: "webp" };
    const start = (spelled === null ? null : outputOf(spelled)) ?? fallback;
    const choice = (suffix, label) => el("select", { id: `${id}-${suffix}`, "aria-label": label });
    const kind = choice("kind", "Kind");
    const size2 = choice("size", "Size");
    const format2 = choice("format", "Format");
    kind.append(...Object.entries(KINDS).map(([name, lists]) => el("option", { value: name }, lists.label)));
    const offer2 = (select, values, keep) => {
      select.replaceChildren(...values.map((value) => el("option", { value }, value)));
      select.value = values.includes(keep) ? keep : values[0] ?? "";
    };
    const follow = () => {
      if (!isKind(kind.value)) {
        return;
      }
      const lists = KINDS[kind.value];
      offer2(size2, lists.sizes, size2.value);
      offer2(format2, lists.formats, format2.value);
    };
    kind.value = start.kind;
    offer2(size2, KINDS[start.kind].sizes, start.size);
    offer2(format2, KINDS[start.kind].formats, start.format);
    kind.addEventListener("change", follow);
    const drop = el("button", { type: "button", class: "quiet", "aria-label": "Remove output" }, "Remove");
    const row2 = el("div", { class: "actions" }, kind, size2, format2, drop);
    drop.addEventListener("click", () => remove(row2));
    return { row: row2, read: () => `${kind.value}/${size2.value}/${format2.value}` };
  }
  function ruleEditor(base, rule2, remove) {
    made += 1;
    const id = `${base}-${made}`;
    const name = field(`${id}-id`, "Rule ID", { spellcheck: "false", autocomplete: "off", required: "" });
    const types = field(`${id}-types`, "Content types", { spellcheck: "false", autocomplete: "off", required: "" }, "Comma-separated, such as image/* or video/mp4.");
    const prefix = field(`${id}-prefix`, "Key prefix", { spellcheck: "false", autocomplete: "off" }, "Empty: every key in the bucket.");
    const rows = [];
    const outputs = el("div", {});
    const removeOutput = (row2) => {
      const index = rows.findIndex((entry) => entry.row === row2);
      if (index >= 0) {
        rows.splice(index, 1);
      }
      row2.remove();
      addOutput.focus();
    };
    const appendOutput = (spelled) => {
      made += 1;
      const entry = outputRow(`${id}-out-${made}`, spelled, removeOutput);
      rows.push(entry);
      outputs.append(entry.row);
    };
    const addOutput = el("button", { type: "button", class: "quiet" }, "Add output");
    addOutput.addEventListener("click", () => {
      appendOutput(null);
      rows.at(-1)?.row.querySelector("select")?.focus();
    });
    (rule2?.outputs ?? [spell({ kind: "image", size: "320", format: "jpeg" })]).forEach(appendOutput);
    name.input.value = rule2?.id ?? "";
    types.input.value = rule2?.content_types.join(", ") ?? "";
    prefix.input.value = rule2?.prefix ?? "";
    const drop = el("button", { type: "button", class: "quiet" }, "Remove rule");
    const box2 = el(
      "fieldset",
      { class: "card" },
      el("legend", {}, "Rule"),
      name.row,
      types.row,
      prefix.row,
      el("fieldset", { class: "checks" }, el("legend", {}, "Outputs: kind, size, format"), outputs, addOutput),
      el("div", { class: "actions" }, drop)
    );
    const editor = {
      box: box2,
      read: () => ({
        id: name.input.value.trim(),
        content_types: typesOf(types.input.value),
        prefix: prefix.input.value,
        outputs: rows.map((entry) => entry.read())
      })
    };
    drop.addEventListener("click", () => remove(editor));
    return editor;
  }
  function mediaForm(screen, bucket2, close) {
    const base = `media-${bucket2.name}`;
    const said = el("div", { "aria-live": "polite" });
    const body = el("div", {}, el("p", { class: "muted" }, "Reading the media rules…"));
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": `${base}-title` },
      el("p", { id: `${base}-title` }, el("strong", {}, "Media: "), mono(bucket2.name)),
      el("p", { class: "muted" }, "An upload whose content type and key match a rule gets the rule's renditions, written under the key's .renditions/ folder. Image sizes are the long side in pixels, video sizes the shorter side. Without rules nothing is processed."),
      body,
      said
    );
    form.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        close();
      }
    });
    const route = `/buckets/${encodeURIComponent(bucket2.name)}/media`;
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
      const editors = [];
      const list2 = el("div", {});
      const none = el("p", { class: "muted" }, "No rules: nothing in this bucket is processed.");
      const show = () => {
        none.hidden = editors.length > 0;
      };
      const remove = (editor) => {
        editors.splice(editors.indexOf(editor), 1);
        editor.box.remove();
        show();
        add.focus();
      };
      const append = (rule2) => {
        const editor = ruleEditor(base, rule2, remove);
        editors.push(editor);
        list2.append(editor.box);
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
      fill(body, none, list2, el("div", { class: "actions" }, add), reason.row, el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Save rules"), cancel));
      form.addEventListener("submit", async (event) => {
        event.preventDefault();
        const why = reason.input.value.trim();
        const rules = editors.map((editor) => editor.read());
        const saved = await call("PUT", route, ignored, why === "" ? { rules } : { rules, reason: why });
        if (saved.ok) {
          announce(`Saved the media rules of ${bucket2.name}.`);
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

  // src/buckets.ts
  //! Buckets: find one (the list is complete, so filtering it here hides nothing
  //! the server sent), see how many objects and bytes each holds as last measured,
  //! create one from the form the header's button reveals, delete an empty one
  //! with a reason, see and set its media rules, and — for operators — set its quota.
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
    return usage.value.buckets.find((entry) => entry.bucket === bucket2.name) ?? { bucket: bucket2.name, objects: 0, bytes: 0, raw_bytes: 0 };
  }
  var numeric = (text2) => el("span", { class: "numeric" }, text2);
  function bucketRow(screen, bucket2, usage) {
    const remove = iconAction("trash", "Delete…", `Delete bucket ${bucket2.name}`, "danger");
    const upload = screen.may?.issue_upload_keys === true ? iconAction("upload", "Upload key…", `Upload key for ${bucket2.name}`) : null;
    const media = iconAction("image", "Media…", `Media rules of ${bucket2.name}`);
    const settings = iconAction("sliders", "Settings…", `Settings of ${bucket2.name}`);
    const held = figures(usage, bucket2);
    const line = row(
      el(
        "span",
        {},
        el("a", { class: "name", href: format({ kind: "objects", bucket: bucket2.name, prefix: "", cursor: null, key: null, filter: "" }) }, icon("buckets"), mono(bucket2.name)),
        encryptionChip(bucket2)
      ),
      numeric(held === null ? "—" : amount(held.objects)),
      numeric(held === null ? "—" : size(held.bytes)),
      numeric(held === null ? "—" : size(held.raw_bytes)),
      numeric(limits(bucket2)),
      moment(bucket2.created),
      bucket2.acknowledgement === "region" ? el("span", {}, mono(bucket2.region), " · region majority") : mono(bucket2.region),
      el("div", { class: "actions" }, upload, media, settings, remove)
    );
    settings.addEventListener("click", () => {
      const cell = el("td", { colspan: "8" });
      const region = el("tr", { class: "asking" }, cell);
      const close = () => {
        region.remove();
        settings.focus();
      };
      const operate = screen.may?.operate === true;
      cell.append(
        el(
          "div",
          { class: "settings" },
          encryptionForm(screen, bucket2, close),
          operate ? quotaForm(screen, bucket2, close) : null,
          operate ? acknowledgementForm(screen, bucket2, close) : null
        )
      );
      line.after(region);
    });
    media.addEventListener("click", () => {
      const cell = el("td", { colspan: "8" });
      const region = el("tr", { class: "asking" }, cell);
      cell.append(
        mediaForm(screen, bucket2, () => {
          region.remove();
          media.focus();
        })
      );
      line.after(region);
    });
    upload?.addEventListener("click", () => {
      const cell = el("td", { colspan: "8" });
      const region = el("tr", { class: "asking" }, cell);
      cell.append(
        uploadKeyForm(screen, bucket2, () => {
          region.remove();
          upload.focus();
        })
      );
      line.after(region);
    });
    remove.addEventListener("click", () => {
      const cell = el("td", { colspan: "8" });
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
    const listed2 = `${count2.toLocaleString()} on this cluster`;
    if (!usage.ok) {
      return `${listed2} · sizes could not be read`;
    }
    if (usage.value.taken === null) {
      return `${listed2} · sizes not measured yet`;
    }
    const measured = `${listed2} · sizes measured ${moment(usage.value.taken)}`;
    return usage.value.stale ? `${measured} — too old to check quotas against, so each write to a limited bucket counts it on the spot` : measured;
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
    const listed2 = el("div", {});
    const draw2 = () => {
      const wanted = filter.input.value.trim();
      const shown = all2.filter((bucket2) => bucket2.name.includes(wanted));
      fill(
        listed2,
        all2.length === 0 ? empty("No buckets yet. Create one with “New bucket”, or with any S3 client.") : shown.length === 0 ? empty(`No bucket name contains “${wanted}”.`) : table("Buckets", ["Name", "Objects", "Size", "On drives", "Limit", "Created", "Region", "Actions"], shown.map((bucket2) => bucketRow(screen, bucket2, usage)))
      );
    };
    filter.input.addEventListener("input", draw2);
    draw2();
    fill(
      screen.main,
      head(TITLE2, measuredLine(all2.length, usage), opener),
      form,
      el("section", { class: "card flush" }, all2.length === 0 ? null : el("div", { class: "toolbar" }, filter.row), listed2)
    );
  }

  // src/members-model.ts
  //! Cluster membership as the console sees it: every member and tombstone with
  //! its pin and whether a join token waits, the token answer shown once, and the
  //! fingerprint an operator types to pin a node to one certificate.
  var HEX64 = /^[0-9a-f]{64}$/;
  var textOrNull2 = (value) => value === null || text(value);
  var memberState = (value) => value === "approved" || value === "removed";
  function readMember(v) {
    if (!record(v) || !text(v["node"]) || !memberState(v["state"]) || !textOrNull2(v["fingerprint"]) || !flag(v["awaiting_token"]) || !textOrNull2(v["token_expires"]) || !text(v["reason"]) || !text(v["by"]) || !text(v["at"])) {
      return null;
    }
    return {
      node: v["node"],
      state: v["state"],
      fingerprint: v["fingerprint"],
      awaiting_token: v["awaiting_token"],
      token_expires: v["token_expires"],
      reason: v["reason"],
      by: v["by"],
      at: v["at"]
    };
  }
  function readMembers(v) {
    return record(v) ? list(v["members"], readMember) : null;
  }
  function readToken(v) {
    if (!record(v) || !text(v["node"]) || !text(v["token"]) || !HEX64.test(v["token"]) || !text(v["expires"])) {
      return null;
    }
    return { node: v["node"], token: v["token"], expires: v["expires"] };
  }
  function isFingerprint(typed) {
    return HEX64.test(typed.trim());
  }

  // src/cluster-members.ts
  //! Cluster admission: every node an operator admitted or removed, with its
  //! pinned certificate and whether a join token waits; and, for an operator, the
  //! forms that admit a node, issue a one-time join token (shown once, here, and
  //! nowhere else) and remove a node for good. The server refuses everyone else
  //! and wants a reason and a recent sign-in for every change.
  var BASE = "/cluster/members";
  var LIFETIMES2 = [
    ["10 minutes", 600],
    ["1 hour", 3600],
    ["1 day", 86400]
  ];
  function done(screen, failure, said) {
    if (failure === null) {
      announce(said);
      screen.redraw();
      return null;
    }
    if (failure.status === 401) {
      screen.signIn();
      return null;
    }
    return failure;
  }
  function stateChip(member2) {
    if (member2.state === "removed") {
      return el("span", { class: "chip bad" }, "Removed");
    }
    return member2.awaiting_token ? el("span", { class: "chip warn" }, "Awaiting token") : el("span", { class: "chip ok" }, "Admitted");
  }
  function certificate(member2) {
    if (member2.fingerprint !== null) {
      return el("code", { title: member2.fingerprint }, `${member2.fingerprint.slice(0, 16)}…`);
    }
    if (member2.state === "removed") {
      return "—";
    }
    return member2.awaiting_token && member2.token_expires !== null ? `Token lapses ${moment(member2.token_expires)}` : "Any the cluster issued for it";
  }
  function removeButton(screen, member2, region) {
    const button = el("button", { type: "button", class: "danger" }, "Remove…");
    button.addEventListener(
      "click",
      () => fill(
        region,
        confirmation(
          `member-remove-${member2.node}`,
          mono(member2.node),
          "The node is removed for good: it is never admitted again and its registration is deleted. Refused while the current layout places shards on it — apply a layout without it first.",
          "Remove",
          async (reason) => {
            const answer = await call("POST", `${BASE}/remove`, ignored, { node: member2.node, reason });
            return done(screen, answer.ok ? null : answer, `Removed ${member2.node}.`);
          },
          () => fill(region)
        )
      )
    );
    return button;
  }
  function memberRows(screen, members, region) {
    if (members.length === 0) {
      return empty("No node has been admitted yet.");
    }
    const operate = screen.may?.operate === true;
    const head2 = ["Node", "State", "Certificate", "Reason", "By", "When", ...operate ? [""] : []];
    return table(
      "Cluster admission",
      head2,
      members.map(
        (member2) => row(
          mono(member2.node),
          stateChip(member2),
          certificate(member2),
          member2.reason,
          member2.by,
          moment(member2.at),
          ...operate ? [member2.state === "approved" ? removeButton(screen, member2, region) : ""] : []
        )
      )
    );
  }
  function reasonField(id) {
    return el("textarea", { id, rows: "2", maxlength: "500", required: "" });
  }
  function admitForm(screen) {
    const node = field("admit-node", "Node id", { autocomplete: "off", spellcheck: "false", required: "" });
    const print = field("admit-fingerprint", "Certificate fingerprint (optional)", { autocomplete: "off", spellcheck: "false" }, "The SHA-256 of the one certificate the node may present, 64 hex digits. Empty: any certificate the cluster issued for this node.");
    const reason = reasonField("admit-reason");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": "admit-title" },
      el("p", { id: "admit-title" }, el("strong", {}, "Admit a node")),
      node.row,
      print.row,
      el("div", { class: "field" }, el("label", { for: reason.id }, "Reason"), reason),
      el("div", { class: "actions" }, el("button", { type: "submit" }, "Admit")),
      said
    );
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const typed = print.input.value.trim();
      if (typed !== "" && !isFingerprint(typed)) {
        print.input.setAttribute("aria-invalid", "true");
        fill(said, el("p", { class: "message bad", role: "alert" }, "A fingerprint is 64 lowercase hex digits."));
        return;
      }
      print.input.removeAttribute("aria-invalid");
      const body = { node: node.input.value.trim(), fingerprint: typed === "" ? null : typed, reason: reason.value.trim() };
      const answer = await call("POST", `${BASE}/approve`, readMember, body);
      const failure = done(screen, answer.ok ? null : answer, `Admitted ${body.node}.`);
      if (failure !== null) {
        fill(said, refusal(failure));
      }
    });
    return form;
  }
  function shownOnce2(node, token, expires, done5) {
    const copy = el("button", { type: "button" }, "Copy token");
    const finish = el("button", { type: "button", class: "primary" }, "I have stored it");
    const said = el("p", { class: "hint", "aria-live": "polite" });
    copy.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(token);
        said.textContent = "Copied.";
      } catch {
        said.textContent = "Copying is not allowed here; select the token and copy it by hand.";
      }
    });
    finish.addEventListener("click", done5);
    queueMicrotask(() => copy.focus());
    return el(
      "div",
      { class: "once", role: "alert" },
      el("p", {}, el("strong", {}, "Store this token now. "), "It is shown once; the server keeps only its digest."),
      el("dl", {}, el("dt", {}, "Node"), el("dd", {}, mono(node)), el("dt", {}, "Lapses"), el("dd", {}, moment(expires)), el("dt", {}, "Join token"), el("dd", {}, mono(token))),
      el("div", { class: "actions" }, copy, finish),
      said
    );
  }
  function tokenForm(screen) {
    const node = field("token-node", "Node id", { autocomplete: "off", spellcheck: "false", required: "" });
    const lifetime = el("select", { id: "token-lifetime" }, ...LIFETIMES2.map(([label, secs]) => el("option", { value: String(secs) }, label)));
    const reason = reasonField("token-reason");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": "token-title" },
      el("p", { id: "token-title" }, el("strong", {}, "Issue a join token")),
      el("p", { class: "muted" }, "Until the node offers the token, no certificate is accepted for it; the first certificate to offer it becomes the node's."),
      node.row,
      el("div", { class: "field" }, el("label", { for: lifetime.id }, "Lapses after"), lifetime),
      el("div", { class: "field" }, el("label", { for: reason.id }, "Reason"), reason),
      el("div", { class: "actions" }, el("button", { type: "submit" }, "Issue")),
      said
    );
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const body = { node: node.input.value.trim(), lifetime_secs: Number(lifetime.value), reason: reason.value.trim() };
      const answer = await call("POST", `${BASE}/token`, readToken, body);
      if (!answer.ok) {
        fill(said, refusal(answer));
        return;
      }
      const { token, expires } = answer.value;
      announce(`Issued a join token for ${body.node}. Store it now.`);
      fill(said, shownOnce2(body.node, token, expires, () => screen.redraw()));
    });
    return form;
  }
  async function admissionCard(screen) {
    const answer = await call("GET", BASE, readMembers);
    const region = el("div", {});
    const card = el("section", { class: "card" }, el("h2", {}, "Admission"));
    if (answer.ok) {
      card.append(memberRows(screen, answer.value, region), region);
    } else {
      card.append(refusal(answer));
    }
    if (screen.may?.operate === true) {
      card.append(admitForm(screen), tokenForm(screen));
    }
    return card;
  }

  // src/layout-model.ts
  //! A layout change as the console sees it: the staged layout (what applying it
  //! would make the next version) and the sets an operator types to stage one —
  //! one erasure set per line, its node ids separated by spaces or commas.
  function readStaged(v) {
    if (!record(v) || !text(v["id"]) || !count(v["base"])) {
      return null;
    }
    const next2 = readLayout(v["layout"]);
    return next2 === null ? null : { id: v["id"], base: v["base"], layout: next2 };
  }
  function readLayout(v) {
    return layout(v) ?? null;
  }
  function parseSets(text2) {
    const lines = text2.split("\n").filter((line) => line.trim() !== "");
    const sets2 = lines.map((line) => line.split(/[\s,]+/).filter((node) => node !== ""));
    return sets2.length === 0 || sets2.some((set) => set.length === 0) ? null : sets2;
  }

  // src/cluster-layout.ts
  //! Changing the layout: the staged layout, if any, with Apply and Discard, and
  //! the form that stages one — one erasure set per line. Applying names the
  //! staging this page showed, so one staged again meanwhile is refused rather
  //! than applied unseen. Only an operator is offered the controls; the server
  //! refuses everyone else and wants a reason and a recent sign-in either way.
  var STAGED = "/cluster/layout/staged";
  function sets(layout2) {
    return layout2.sets.map(
      (set, i) => el("div", {}, el("h3", {}, `Erasure set ${i + 1}`), el("ol", {}, ...set.map((member2) => el("li", {}, mono(member2.node), member2.zone === null ? "" : el("span", { class: "muted" }, ` — zone ${member2.zone}`)))))
    );
  }
  function done2(screen, failure, said) {
    if (failure === null) {
      announce(said);
      screen.redraw();
      return null;
    }
    if (failure.status === 401) {
      screen.signIn();
      return null;
    }
    return failure;
  }
  function stagedPart(screen, staged) {
    const { id, base, layout: layout2 } = staged;
    const region = el("div", {});
    const apply = el("button", { type: "button", class: "primary" }, "Apply…");
    const discard = el("button", { type: "button" }, "Discard…");
    const close = () => fill(region);
    apply.addEventListener(
      "click",
      () => fill(
        region,
        confirmation(
          "layout-apply",
          el("span", {}, `version ${layout2.version}, ${layout2.sets.length} erasure set${layout2.sets.length === 1 ? "" : "s"}`),
          "New objects are written under this layout from now on; objects already written keep the layout they were written under. Refused if the layout or this staging changed since the page showed it.",
          "Apply",
          async (reason) => {
            const answer = await call("POST", "/cluster/layout/apply", readLayout, { id, reason });
            return done2(screen, answer.ok ? null : answer, `Applied layout version ${layout2.version}.`);
          },
          close
        )
      )
    );
    discard.addEventListener(
      "click",
      () => fill(
        region,
        confirmation(
          "layout-discard",
          el("span", {}, `the layout staged over version ${base}`),
          "The staged layout is removed; the cluster does not change.",
          "Discard",
          async (reason) => {
            const answer = await call("DELETE", STAGED, ignored, { reason });
            return done2(screen, answer.ok ? null : answer, "Discarded the staged layout.");
          },
          close
        )
      )
    );
    return el(
      "div",
      {},
      el("p", { class: "muted" }, `Staged over version ${base}; applying it makes it version ${layout2.version}.`),
      ...sets(layout2),
      screen.may?.operate === true ? el("div", { class: "actions" }, apply, discard) : null,
      region
    );
  }
  function stageForm(screen) {
    const nodes = el("textarea", { id: "layout-sets", rows: "4", autocomplete: "off", spellcheck: "false", "aria-describedby": "layout-sets-hint" });
    const reason = el("textarea", { id: "layout-reason", rows: "2", maxlength: "500", required: "", "aria-describedby": "layout-reason-hint" });
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": "layout-stage-title" },
      el("p", { id: "layout-stage-title" }, el("strong", {}, "Stage a layout")),
      el("div", { class: "field" }, el("label", { for: nodes.id }, "Erasure sets"), nodes, el("p", { id: "layout-sets-hint", class: "hint" }, "One set per line: its node ids, one per shard, separated by spaces or commas.")),
      el("div", { class: "field" }, el("label", { for: reason.id }, "Reason"), reason, el("p", { id: "layout-reason-hint", class: "hint" }, "Required. Recorded with your key id.")),
      el("div", { class: "actions" }, el("button", { type: "submit" }, "Stage")),
      said
    );
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const typed = parseSets(nodes.value);
      if (typed === null) {
        nodes.setAttribute("aria-invalid", "true");
        fill(said, el("p", { class: "message bad", role: "alert" }, "Write at least one set, each line naming its nodes."));
        return;
      }
      nodes.removeAttribute("aria-invalid");
      const answer = await call("PUT", STAGED, readStaged, { sets: typed, reason: reason.value.trim() });
      const failure = done2(screen, answer.ok ? null : answer, "Staged a layout.");
      if (failure !== null) {
        fill(said, refusal(failure));
      }
    });
    return form;
  }
  async function stagingCard(screen) {
    const answer = await call("GET", STAGED, readStaged);
    const card = el("section", { class: "card" }, el("h2", {}, "Layout change"));
    if (answer.ok) {
      card.append(stagedPart(screen, answer.value));
    } else if (answer.code === "nothing_staged") {
      card.append(el("p", { class: "muted" }, "Nothing is staged."));
    } else {
      card.append(refusal(answer));
    }
    if (screen.may?.operate === true) {
      card.append(stageForm(screen));
    }
    return card;
  }

  // src/regions-model.ts
  //! Region failover as the console sees it: each region an operator ever failed
  //! over, under which epoch, and where its writes go while it is failed over.
  var REGION = /^[a-z][a-z0-9]*(-[a-z0-9]+)*$/;
  function readRegion(v) {
    if (!record(v) || !text(v["region"]) || !count(v["epoch"]) || !flag(v["failed_over"]) || !(v["promoted"] === null || text(v["promoted"])) || !text(v["reason"]) || !text(v["by"]) || !text(v["at"])) {
      return null;
    }
    if (v["failed_over"] !== (v["promoted"] !== null)) {
      return null;
    }
    return {
      region: v["region"],
      epoch: v["epoch"],
      failed_over: v["failed_over"],
      promoted: v["promoted"],
      reason: v["reason"],
      by: v["by"],
      at: v["at"]
    };
  }
  function readRegions(v) {
    return record(v) ? list(v["regions"], readRegion) : null;
  }
  function isRegionName(typed) {
    return REGION.test(typed.trim());
  }

  // src/cluster-regions.ts
  //! Region failover: every region an operator failed over, under which epoch and
  //! to where; and, for an operator, failing a region over to another and
  //! restoring it. A failed-over region refuses client writes and still takes the
  //! replicas that catch it up. The server wants a reason and a recent sign-in for
  //! every change, and records each attempt.
  var BASE2 = "/regions";
  function done3(screen, failure, said) {
    if (failure === null) {
      announce(said);
      screen.redraw();
      return null;
    }
    if (failure.status === 401) {
      screen.signIn();
      return null;
    }
    return failure;
  }
  function stateChip2(region) {
    return region.failed_over ? el("span", { class: "chip bad" }, "Failed over") : el("span", { class: "chip ok" }, "Taking writes");
  }
  function restoreButton(screen, region, area) {
    const button = el("button", { type: "button" }, "Restore…");
    button.addEventListener(
      "click",
      () => fill(
        area,
        confirmation(
          `region-restore-${region.region}`,
          mono(region.region),
          "The region takes client writes again, under the next epoch. Restore it once it has caught up from the promoted region and clients may write here again.",
          "Restore",
          async (reason) => {
            const answer = await call("POST", `${BASE2}/restore`, ignored, { region: region.region, reason });
            return done3(screen, answer.ok ? null : answer, `Restored ${region.region}.`);
          },
          () => fill(area)
        )
      )
    );
    return button;
  }
  function regionRows(screen, regions, area) {
    if (regions.length === 0) {
      return empty("No region has been failed over.");
    }
    const operate = screen.may?.operate === true;
    const head2 = ["Region", "State", "Writes go to", "Epoch", "Reason", "By", "When", ...operate ? [""] : []];
    return table(
      "Region failover",
      head2,
      regions.map(
        (region) => row(
          mono(region.region),
          stateChip2(region),
          region.promoted === null ? "—" : mono(region.promoted),
          String(region.epoch),
          region.reason,
          region.by,
          moment(region.at),
          ...operate ? [region.failed_over ? restoreButton(screen, region, area) : ""] : []
        )
      )
    );
  }
  function failOverForm(screen, area) {
    const failed2 = field("failover-region", "Region to fail over", { autocomplete: "off", spellcheck: "false", required: "" });
    const promoted = field("failover-promoted", "Region its writes go to", { autocomplete: "off", spellcheck: "false", required: "" }, "A region holding replica buckets of this one's.");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": "failover-title" },
      el("p", { id: "failover-title" }, el("strong", {}, "Fail a region over")),
      failed2.row,
      promoted.row,
      el("div", { class: "actions" }, el("button", { type: "submit", class: "danger" }, "Fail over…")),
      said
    );
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const [from, to] = [failed2.input.value.trim(), promoted.input.value.trim()];
      for (const [input, value] of [[failed2.input, from], [promoted.input, to]]) {
        input.toggleAttribute("aria-invalid", !isRegionName(value));
      }
      if (!isRegionName(from) || !isRegionName(to)) {
        fill(said, el("p", { class: "message bad", role: "alert" }, "A region is lowercase letters, digits and single hyphens, beginning with a letter."));
        return;
      }
      if (from === to) {
        fill(said, el("p", { class: "message bad", role: "alert" }, "A region is failed over to another region."));
        return;
      }
      fill(said);
      fill(
        area,
        confirmation(
          "region-failover",
          el("span", {}, mono(from), " → ", mono(to)),
          `Every node of ${from} refuses client writes until it is restored; reads go on, and replicas from ${to} still arrive. Point clients at ${to}'s buckets.`,
          "Fail over",
          async (reason) => {
            const answer = await call("POST", `${BASE2}/failover`, ignored, { region: from, promoted: to, reason });
            return done3(screen, answer.ok ? null : answer, `Failed ${from} over to ${to}.`);
          },
          () => fill(area)
        )
      );
    });
    return form;
  }
  async function regionsCard(screen) {
    const answer = await call("GET", BASE2, readRegions);
    const area = el("div", {});
    const card = el("section", { class: "card" }, el("h2", {}, "Regions"));
    if (answer.ok) {
      card.append(regionRows(screen, answer.value, area), area);
    } else {
      card.append(refusal(answer));
    }
    if (screen.may?.operate === true) {
      card.append(failOverForm(screen, area));
    }
    return card;
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

  // src/encryption-model.ts
  //! This node's encryption keys as the console sees them: not yet created,
  //! sealed, or held — and then which key new objects are wrapped under, and
  //! the keys operators created by name for `aws:kms`.
  function readBehind(v) {
    if (record(v) && text(v["id"]) && count(v["remaining"])) {
      return { id: v["id"], remaining: v["remaining"] };
    }
    return null;
  }
  function readKeys2(v) {
    if (!record(v)) {
      return null;
    }
    const state2 = v["state"];
    if (state2 === "uninitialised" || state2 === "sealed") {
      return { state: state2 };
    }
    const named = list(v["named"], (name) => text(name) ? name : null);
    const behind = list(v["behind"], readBehind);
    if (state2 === "unsealed" && text(v["active"]) && count(v["keys"]) && named !== null && behind !== null) {
      return { state: state2, active: v["active"], keys: v["keys"], named, behind };
    }
    return null;
  }

  // src/encryption.ts
  //! The encryption keys of this node, for an operator: whether they are held,
  //! the form that unseals them with the keys passphrase (the first unseal
  //! creates them), sealing them again, and the keys created by name that
  //! objects can be sealed under with `aws:kms`, and rotating the server key. Each node holds its own keys,
  //! so this acts on the node serving the console; other nodes hold a new named
  //! key after their next unseal. The server wants a recent sign-in and records
  //! every attempt without the passphrase.
  var BASE3 = "/encryption";
  function done4(screen, failure, said) {
    if (failure === null) {
      announce(said);
      screen.redraw();
      return null;
    }
    if (failure.status === 401) {
      screen.signIn();
      return null;
    }
    return failure;
  }
  function stateLine(keys2) {
    switch (keys2.state) {
      case "uninitialised":
        return el("p", {}, el("span", { class: "chip warn" }, "Not created"), " The first unseal creates the keys under the passphrase you give.");
      case "sealed":
        return el("p", {}, el("span", { class: "chip bad" }, "Sealed"), " This node cannot read or write encrypted objects until the keys are unsealed.");
      case "unsealed":
        return el("p", {}, el("span", { class: "chip ok" }, "Unsealed"), " New objects are wrapped under ", mono(keys2.active), `; ${keys2.keys.toLocaleString()} held.`);
    }
  }
  function unsealForm(screen, keys2) {
    const first = keys2.state === "uninitialised";
    const hint = first ? "At least 15 characters, without quotes or backslashes. It becomes the keys passphrase: there is no recovering it." : "The keys passphrase. Wrong ones in a row make the next try wait.";
    const passphrase = field("keys-passphrase", "Keys passphrase", { type: "password", autocomplete: "off", required: "" }, hint);
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm", novalidate: "", "aria-labelledby": "keys-unseal-title" },
      el("p", { id: "keys-unseal-title" }, el("strong", {}, first ? "Create and unseal the keys" : "Unseal the keys")),
      passphrase.row,
      el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Unseal")),
      said
    );
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const answer = await call("POST", `${BASE3}/unseal`, readKeys2, { passphrase: passphrase.input.value });
      passphrase.input.value = "";
      const failure = done4(screen, answer.ok ? null : answer, "The encryption keys are unsealed on this node.");
      if (failure !== null) {
        fill(said, refusal(failure));
      }
    });
    return form;
  }
  function namedKeys(keys2) {
    if (keys2.length === 0) {
      return el("p", {}, "No named keys yet. A request asking for ", mono("aws:kms"), " without a key id uses the key named ", mono("aws-s3"), ".");
    }
    return el("p", {}, "Named keys: ", ...keys2.flatMap((name, at2) => at2 === 0 ? [mono(name)] : [", ", mono(name)]), ".");
  }
  function createForm2(screen) {
    const name = field("key-name", "Key name", { autocomplete: "off", required: "", pattern: "[a-z0-9-]{1,64}" }, "1 to 64 of a-z, 0-9 and -. Names are visible to anyone who can list the keys vault; keys are never replaced.");
    const passphrase = field("key-passphrase", "Keys passphrase", { type: "password", autocomplete: "off", required: "" }, "Opens the keys vault for this one write.");
    const why = field("key-reason", "Reason", { autocomplete: "off" }, "Kept in the action record.");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm card-part", novalidate: "", "aria-labelledby": "keys-create-title" },
      el("p", { id: "keys-create-title" }, el("strong", {}, "Create a named key")),
      name.row,
      passphrase.row,
      why.row,
      el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Create")),
      said
    );
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const reason = why.input.value.trim();
      const answer = await call("POST", `${BASE3}/keys`, readKeys2, {
        name: name.input.value.trim(),
        passphrase: passphrase.input.value,
        ...reason === "" ? {} : { reason }
      });
      passphrase.input.value = "";
      const failure = done4(screen, answer.ok ? null : answer, "The key is created and held on this node.");
      if (failure !== null) {
        fill(said, refusal(failure));
      }
    });
    return form;
  }
  function rotateForm(screen, keys2) {
    const passphrase = field("rotate-passphrase", "Keys passphrase", { type: "password", autocomplete: "off", required: "" }, "Opens the keys vault for this one write.");
    const why = field("rotate-reason", "Reason", { autocomplete: "off" }, "Kept in the action record.");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm card-part", novalidate: "", "aria-labelledby": "keys-rotate-title" },
      el("p", { id: "keys-rotate-title" }, el("strong", {}, "Rotate the server key")),
      el("p", {}, "New objects are wrapped under a new key in place of ", mono(keys2.active), "; objects under older keys still open. Other nodes take the new key at their next unseal."),
      passphrase.row,
      why.row,
      el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Rotate")),
      said
    );
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const reason = why.input.value.trim();
      const answer = await call("POST", `${BASE3}/rotate`, readKeys2, {
        passphrase: passphrase.input.value,
        ...reason === "" ? {} : { reason }
      });
      passphrase.input.value = "";
      const failure = done4(screen, answer.ok ? null : answer, "A new server key is active on this node.");
      if (failure !== null) {
        fill(said, refusal(failure));
      }
    });
    return form;
  }
  function destroyForm(screen, key) {
    const passphrase = field(`destroy-${key.id}-passphrase`, "Keys passphrase", { type: "password", autocomplete: "off", required: "" }, "Opens the keys vault for this one write.");
    const why = field(`destroy-${key.id}-reason`, "Reason", { autocomplete: "off" }, "Kept in the action record.");
    const said = el("div", { "aria-live": "polite" });
    const form = el(
      "form",
      { class: "confirm card-part", novalidate: "" },
      el("p", {}, "No record names ", mono(key.id), " any more. Destroying it removes it from the keys vault; backups taken before still hold it until they expire."),
      passphrase.row,
      why.row,
      el("div", { class: "actions" }, el("button", { type: "submit", class: "danger" }, `Destroy ${key.id}`)),
      said
    );
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const reason = why.input.value.trim();
      const answer = await call("POST", `${BASE3}/destroy`, readKeys2, {
        key: key.id,
        passphrase: passphrase.input.value,
        ...reason === "" ? {} : { reason }
      });
      passphrase.input.value = "";
      const failure = done4(screen, answer.ok ? null : answer, `${key.id} is destroyed.`);
      if (failure !== null) {
        fill(said, refusal(failure));
      }
    });
    return form;
  }
  function olderKeys(screen, behind) {
    return behind.map(
      (key) => key.remaining === 0 ? destroyForm(screen, key) : el("p", {}, mono(key.id), ` still wraps ${key.remaining.toLocaleString()} data keys; the re-wrap moves them to the active key.`)
    );
  }
  function sealButton(screen, region) {
    const button = el("button", { type: "button", class: "danger" }, "Seal…");
    button.addEventListener(
      "click",
      () => fill(
        region,
        confirmation(
          "keys-seal",
          el("span", {}, "the encryption keys on this node"),
          "This node forgets its keys: encrypted objects are refused here until someone unseals them with the passphrase.",
          "Seal",
          async (reason) => {
            const answer = await call("POST", `${BASE3}/seal`, readKeys2, { reason });
            return done4(screen, answer.ok ? null : answer, "The encryption keys are sealed on this node.");
          },
          () => fill(region)
        )
      )
    );
    return button;
  }
  async function encryptionCard(screen) {
    if (screen.may?.operate !== true) {
      return null;
    }
    const card = el("section", { class: "card" }, el("h2", {}, "Encryption keys"));
    const answer = await call("GET", BASE3, readKeys2);
    if (!answer.ok) {
      card.append(refusal(answer));
      return card;
    }
    const keys2 = answer.value;
    const region = el("div", {});
    card.append(stateLine(keys2));
    if (keys2.state === "unsealed") {
      card.append(namedKeys(keys2.named), el("div", { class: "actions" }, sealButton(screen, region)), region, createForm2(screen), rotateForm(screen, keys2), ...olderKeys(screen, keys2.behind));
    } else {
      card.append(unsealForm(screen, keys2));
    }
    return card;
  }

  // src/status.ts
  //! The overview: is this node and its cluster healthy, how much is stored and
  //! how full the drives are, and is anything waiting to be healed. Each tile
  //! links to where the operator acts on it.
  var TITLE3 = "Overview";
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
    if (heal.at_risk > 0) {
      const risk = `${heal.at_risk.toLocaleString()} at risk`;
      return tile("pulse", "Healing", counted, el("span", {}, el("span", { class: "chip bad" }, risk), " Healing keeps failing for these: too few shards can be read."));
    }
    return tile("pulse", "Healing", counted, el("span", {}, el("span", { class: "chip warn" }, "Waiting"), " Members heal these objects in the background."));
  }
  function replicationTile(replication) {
    if (replication === null) {
      return null;
    }
    if (replication.failed > 0) {
      const failed2 = `${amount(replication.failed)} failed`;
      return tile("layers", "Replication", amount(replication.backlog), el("span", {}, el("span", { class: "chip bad" }, failed2), " Given up after 10 tries; these versions are FAILED."));
    }
    if (replication.backlog === 0) {
      return tile("layers", "Replication", el("span", { class: "chip ok" }, "Caught up"), "Every replicated version has reached its destination.");
    }
    return tile("layers", "Replication", amount(replication.backlog), el("span", {}, el("span", { class: "chip warn" }, `${amount(replication.in_flight)} sending`), " Versions waiting to reach their destination region."));
  }
  function state(member2) {
    return member2.answering ? el("span", { class: "chip ok" }, "Answering") : el("span", { class: "chip bad" }, "Not answering");
  }
  function memberTable(members) {
    return members.length === 0 ? empty("No members are registered yet.") : table(
      "Cluster members",
      ["Member", "Zone", "State", "Disk", "Internal address"],
      members.map(
        (member2) => row(
          mono(member2.node),
          member2.zone === null ? "—" : mono(member2.zone),
          state(member2),
          member2.drive === null ? "—" : meter(member2.drive, `Disk space used on ${member2.node}`),
          mono(member2.endpoint)
        )
      )
    );
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
    return tile(
      "buckets",
      "Stored",
      size(measured.bytes),
      `${objects2} · ${size(measured.raw_bytes)} on the drives · measured ${moment(measured.taken)}`
    );
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
    loading(screen, TITLE3, "the node's status");
    const [answer, usage, encryption] = await Promise.all([call("GET", "/status", readStatus), call("GET", "/usage", readUsage), encryptionCard(screen)]);
    if (!answer.ok) {
      failed(screen, TITLE3, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const node = answer.value;
    const members = node.members === null ? null : memberTable(node.members);
    fill(
      screen.main,
      head(TITLE3, el("span", {}, "Region ", mono(node.region), " · version ", mono(node.version))),
      el(
        "div",
        { class: "tiles" },
        tile("node", "Node", node.node === null ? "Single" : mono(node.node), node.node === null ? "Not a cluster member" : "This node's name in the cluster"),
        membersTile(node.members),
        tile("layers", "Erasure code", node.erasure === null ? "None" : mono(node.erasure), node.erasure === null ? "Whole objects on one node" : "Data + parity shards per object"),
        healing(node.heal_backlog),
        replicationTile(node.replication),
        storedTile(usage),
        diskTile(node),
        tile("buckets", "Buckets", "Browse", "Find a bucket or an object", format({ kind: "buckets" })),
        tile("record", "Action record", "Review", "Every console change and download", format({ kind: "actions", before: null }))
      ),
      members === null ? null : el("section", { class: "card flush" }, el("h2", {}, "Members"), members),
      encryption
    );
  }

  // src/cluster.ts
  //! The cluster: its members with whether each answers and its disk, the
  //! layout objects are placed by and its erasure code, the healing backlog,
  //! the metadata nodes, which nodes are admitted, which regions are failed over, and a layout change staged or to stage. Only a key that
  //! may see the cluster is shown this view; the server refuses everyone else.
  var TITLE4 = "Cluster";
  function layoutCard(cluster2) {
    if (cluster2.layout === null) {
      const why = cluster2.erasure === null ? "This node stores objects on its own." : "The layout is fixed once enough members have registered.";
      return el("section", { class: "card" }, el("h2", {}, "Layout"), el("p", { class: "muted" }, why));
    }
    const { version, sets: sets2 } = cluster2.layout;
    return el(
      "section",
      { class: "card" },
      el("h2", {}, "Layout"),
      el(
        "p",
        { class: "muted" },
        `Version ${version}. Each object goes to one erasure set, chosen from its data id, and each node of that set holds one of its shards; no set keeps more shards in one zone than it can lose.`
      ),
      ...sets2.map(
        (set, i) => el("div", {}, el("h3", {}, sets2.length === 1 ? "Erasure set" : `Erasure set ${i + 1}`), el("ol", {}, ...set.map((member2) => el("li", {}, mono(member2.node), member2.zone === null ? "" : el("span", { class: "muted" }, ` — zone ${member2.zone}`)))))
      )
    );
  }
  function moving(waiting) {
    if (waiting.listed === 0) {
      return tile("layers", "Layout moves", el("span", { class: "chip ok" }, "Up to date"), "Every object is on the current layout.");
    }
    const counted = waiting.more ? `> ${waiting.listed.toLocaleString()}` : waiting.listed.toLocaleString();
    return tile("layers", "Layout moves", counted, el("span", {}, el("span", { class: "chip warn" }, "Moving"), " Members move these objects onto the current layout in the background."));
  }
  async function cluster(screen) {
    loading(screen, TITLE4, "the cluster");
    const answer = await call("GET", "/cluster", readCluster);
    if (!answer.ok) {
      failed(screen, TITLE4, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const view = answer.value;
    const staging = view.layout === null ? null : await stagingCard(screen);
    const admission = view.erasure === null ? null : await admissionCard(screen);
    const regions = await regionsCard(screen);
    if (!screen.live()) {
      return;
    }
    fill(
      screen.main,
      head(TITLE4, view.node === null ? "A node on its own" : el("span", {}, "This node: ", mono(view.node))),
      el(
        "div",
        { class: "tiles" },
        tile("layers", "Erasure code", view.erasure === null ? "—" : mono(view.erasure), view.erasure === null ? "Not a cluster member" : "data + parity shards per object"),
        healing(view.erasure === null ? null : view.heal_backlog),
        view.erasure === null ? null : moving(view.moving),
        tile("node", "Metadata nodes", String(view.metadata.addresses.length), el("span", {}, ...view.metadata.addresses.flatMap((address, i) => [i === 0 ? "" : ", ", mono(address)])))
      ),
      view.members === null ? null : el("section", { class: "card flush" }, el("h2", {}, "Members"), memberTable(view.members)),
      admission,
      regions,
      layoutCard(view),
      staging
    );
  }

  // src/object-delete.ts
  //! What a refused delete of one object tells the operator. The delete is
  //! conditional on the ETag they saw, so a refusal usually means the object moved
  //! on — and that is worth saying in words rather than as a status.
  function deleteRefusal(failure) {
    switch (failure.status) {
      case 412:
        return { ...failure, message: "This object changed since you saw it, so it was not deleted." };
      case 404:
        return { ...failure, message: "This object is already gone." };
      default:
        return failure;
    }
  }

  // src/file-kind.ts
  //! What kind of file a key holds, read from the content type its write stored,
  //! so a listing can say "image" or "archive" without opening the object.
  var FILE = { icon: "file", label: "File" };
  var TEXT = { icon: "text", label: "Text" };
  var ARCHIVE = { icon: "archive", label: "Archive" };
  var FAMILIES = {
    image: { icon: "image", label: "Image" },
    video: { icon: "video", label: "Video" },
    audio: { icon: "audio", label: "Audio" },
    text: TEXT
  };
  var READABLE = /* @__PURE__ */ new Set(["application/json", "application/xml", "application/javascript"]);
  var PACKED = /* @__PURE__ */ new Set(["application/zip", "application/gzip", "application/x-gzip", "application/x-tar", "application/x-7z-compressed"]);
  function kindOf(contentType) {
    const essence = (contentType ?? "").split(";")[0]?.trim().toLowerCase() ?? "";
    const [family, subtype = ""] = essence.split("/");
    const named = family === void 0 ? void 0 : FAMILIES[family];
    if (named !== void 0) {
      return named;
    }
    if (family !== "application") {
      return FILE;
    }
    if (READABLE.has(essence) || subtype.endsWith("+json") || subtype.endsWith("+xml")) {
      return TEXT;
    }
    return PACKED.has(essence) ? ARCHIVE : FILE;
  }

  // src/object-facts.ts
  //! An object's stored facts as the console shows them — on the object's own
  //! page and in the listing's side pane. Everything here comes from the object's
  //! record; nothing reads the object's bytes.
  function pairs(caption, values) {
    const entries = Object.entries(values);
    return entries.length === 0 ? el("div", { class: "empty" }, el("p", {}, "None.")) : table(caption, ["Name", "Value"], entries.map(([name, value]) => row(mono(name), mono(value))));
  }
  var retained = (detail) => detail.retention === null ? "None" : `${detail.retention.mode === "COMPLIANCE" ? "Compliance" : "Governance"} until ${moment(detail.retention.until)}`;
  function facts(detail) {
    const type = detail.headers["content-type"] ?? null;
    return el(
      "dl",
      { class: "facts" },
      el("dt", {}, "Type"),
      el("dd", {}, type === null ? "None stored" : el("span", {}, `${kindOf(type).label} · `, mono(type))),
      el("dt", {}, "Size"),
      el("dd", {}, `${size(detail.size)} (${detail.size.toLocaleString()} bytes)`),
      el("dt", {}, "ETag"),
      el("dd", {}, mono(detail.etag)),
      el("dt", {}, "Last modified"),
      el("dd", {}, moment(detail.modified)),
      el("dt", {}, "Upload"),
      el("dd", {}, detail.parts === null ? "Single request" : `Multipart, ${detail.parts} parts`),
      el("dt", {}, "Retention"),
      el("dd", {}, retained(detail)),
      el("dt", {}, "Legal hold"),
      el("dd", {}, detail.legalHold ? "On" : "Off")
    );
  }

  // src/object.ts
  //! One object: what the server holds for it, a recorded download, and a delete
  //! that only goes through while the object is still the version shown here —
  //! the ETag on screen is the condition, so a rewrite in between is refused.
  var TITLE5 = "Object";
  var parent = (key) => key.slice(0, key.lastIndexOf("/") + 1);
  async function object(screen, bucket2, key) {
    loading(screen, TITLE5, "the object");
    const where = `/buckets/${encodeURIComponent(bucket2)}/object`;
    const answer = await call("GET", `${where}${search([["key", key]])}`, readDetail);
    if (!answer.ok) {
      failed(screen, TITLE5, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const detail = answer.value;
    const back = format({ kind: "objects", bucket: bucket2, prefix: parent(key), cursor: null, key, filter: "" });
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
            const done5 = await call("DELETE", `${where}${search([["key", key]])}`, ignored, { etag: detail.etag, reason: why });
            if (done5.ok) {
              announce(`Deleted ${key} from ${bucket2}.`);
              location.hash = back;
              return null;
            }
            if (done5.status === 401) {
              screen.signIn();
              return null;
            }
            return deleteRefusal(done5);
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
      el("section", { class: "card" }, facts(detail)),
      el("section", { class: "card" }, el("h2", {}, "Actions"), reason.row, el("div", { class: "actions" }, download, remove), asking),
      el("section", { class: "card flush" }, el("h2", {}, "Headers"), pairs("Stored headers", detail.headers)),
      el("section", { class: "card flush" }, el("h2", {}, "User metadata"), pairs("User metadata", detail.metadata)),
      el("section", { class: "card flush" }, el("h2", {}, "Tags"), pairs("Tags", detail.tags)),
      el("section", { class: "card flush" }, el("h2", {}, "Checksums"), pairs("Stored checksums", detail.checksums))
    );
  }

  // src/object-actions.ts
  //! A key's row actions in the listing, as icon buttons: its details beside the
  //! list, a download (recorded by the server), the key copied, and a delete that
  //! only goes through while the object is still the version listed — the listed
  //! ETag is the condition, so a rewrite in between is refused.
  function objectActions(screen, bucket2, object2, line, details, gone) {
    const where = `/buckets/${encodeURIComponent(bucket2)}/object`;
    const info = iconAction("info", "Details", `Details of ${object2.key}`);
    const download = iconAction("download", "Download", `Download ${object2.key}`);
    const copy = iconAction("copy", "Copy key", `Copy the key ${object2.key}`);
    const remove = iconAction("trash", "Delete…", `Delete ${object2.key}`, "danger");
    info.addEventListener("click", details);
    download.addEventListener("click", () => {
      location.assign(`${path(`${where}/content`)}${search([["key", object2.key]])}`);
    });
    copy.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(object2.key);
        announce(`Copied the key ${object2.key}.`);
      } catch {
        announce("Copying is not allowed here; open the object's page and copy the key by hand.");
      }
    });
    remove.addEventListener("click", () => {
      const close = open(
        line,
        confirmation(
          `delete-${line.rowIndex}`,
          mono(`${bucket2}/${object2.key}`),
          `There is no versioning: the object is gone for every client. It is deleted only if it is still ETag ${object2.etag}.`,
          "Delete object",
          async (reason) => {
            const done5 = await call("DELETE", `${where}${search([["key", object2.key]])}`, ignored, { etag: object2.etag, reason });
            if (!done5.ok) {
              const shown = settle(screen, done5);
              return shown === null ? null : deleteRefusal(shown);
            }
            close();
            const beside = (line.nextElementSibling ?? line.previousElementSibling)?.querySelector("a.name");
            line.remove();
            gone();
            beside?.focus();
            announce(`Deleted ${object2.key} from ${bucket2}.`);
            return null;
          },
          () => {
            close();
            remove.focus();
          }
        )
      );
    });
    return [info, download, copy, remove];
  }

  // src/object-pages.ts
  //! One run of a listing: its first page shown, and later pages appended from
  //! the server's cursor as the end of the list comes into view or "Load more" is
  //! pressed. There is no total to show, because counting a large bucket is a
  //! scan. A new filter starts a new run and stops the old one.
  function run(screen, first, rowsOf, fetchPage, caption, nothing, back) {
    const rows = rowsOf(first);
    const body = rows.length === 0 ? nothing : table(caption, ["Name", "Type", "Size", "Last modified", "ETag", "Actions"], rows);
    let loaded = rows.length;
    let next2 = first.next;
    let busy = false;
    let stopped = false;
    const status2 = el("span", { role: "status" });
    const more = el("button", { type: "button" }, "Load more");
    const paging = el("p", { class: "paging" }, status2, more, back);
    const settle2 = () => {
      more.hidden = next2 === null;
      const shown = loaded === 1 ? "1 key or folder shown" : `${loaded.toLocaleString()} keys and folders shown`;
      status2.textContent = next2 !== null ? `${shown}.` : loaded === 1 ? `${shown}; there are no more.` : `All ${shown}.`;
    };
    const near = () => paging.isConnected && paging.getBoundingClientRect().top < window.innerHeight * 2;
    const live = () => screen.live() && !stopped;
    const loadMore = async () => {
      if (busy || next2 === null || !live()) {
        return;
      }
      busy = true;
      more.setAttribute("aria-busy", "true");
      status2.textContent = "Loading more keys…";
      const answer = await fetchPage(next2);
      busy = false;
      more.removeAttribute("aria-busy");
      if (!live()) {
        return;
      }
      if (!answer.ok) {
        if (answer.status === 401) {
          screen.signIn();
          return;
        }
        status2.textContent = `${answer.message} Use "Load more" to try again.`;
        return;
      }
      const added = rowsOf(answer.value);
      body.querySelector("tbody")?.append(...added);
      loaded += added.length;
      next2 = answer.value.next;
      settle2();
      if (near()) {
        void loadMore();
      }
    };
    more.addEventListener("click", () => void loadMore());
    const watch = new IntersectionObserver(
      (entries) => {
        if (!live() || next2 === null) {
          watch.disconnect();
        } else if (entries.some((entry) => entry.isIntersecting)) {
          void loadMore();
        }
      },
      { rootMargin: "0px 0px 100% 0px" }
    );
    settle2();
    return {
      nodes: rows.length === 0 && next2 === null && back === null ? [body] : [body, paging],
      start: () => watch.observe(paging),
      stop: () => {
        stopped = true;
        watch.disconnect();
      }
    };
  }

  // src/object-pane.ts
  //! The listing's side pane: one selected object's metadata beside the list, so
  //! the operator keeps their place while looking. It reads the object's record
  //! only — the bytes stay on the drives until somebody downloads them.
  async function describe(screen, pane, bucket2, key, close, current2) {
    const name = key.slice(key.lastIndexOf("/") + 1) || key;
    const shut = iconAction("close", "Close", `Close the details of ${name}`);
    shut.addEventListener("click", close);
    const top = el("div", {}, el("div", { class: "pane-head" }, el("h2", { id: "pane-title" }, "Details"), shut), el("p", { class: "pane-key" }, mono(key)));
    fill(pane, top, el("p", { class: "pane-note", role: "status" }, "Loading the details…"));
    const answer = await call("GET", `/buckets/${encodeURIComponent(bucket2)}/object${search([["key", key]])}`, readDetail);
    if (!screen.live() || !current2()) {
      return;
    }
    if (!answer.ok) {
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      fill(pane, top, el("p", { class: "pane-note", role: "alert" }, answer.status === 404 ? "This object is gone; reload the listing." : answer.message));
      return;
    }
    const detail = answer.value;
    fill(
      pane,
      top,
      facts(detail),
      el("p", { class: "pane-link" }, el("a", { href: format({ kind: "object", bucket: bucket2, key }) }, "Open the object's page"), " to download or delete it."),
      el("h3", {}, "Headers"),
      pairs("Stored headers", detail.headers),
      el("h3", {}, "User metadata"),
      pairs("User metadata", detail.metadata),
      el("h3", {}, "Tags"),
      pairs("Tags", detail.tags),
      el("h3", {}, "Checksums"),
      pairs("Stored checksums", detail.checksums)
    );
  }

  // src/objects.ts
  //! A bucket's keys in the server's byte order, rolled up at `/` like a folder
  //! tree, paged by the server's cursor as the list is scrolled. A filter keeps
  //! the names in the open folder that start with some text — it is a longer
  //! prefix, so the store lists only those. Selecting a key shows its metadata in
  //! a side pane beside the list (the selection is in the URL); the arrow keys
  //! move the selection and Escape closes the pane.
  var PAGE2 = 100;
  var FILTER_PAUSE_MS = 300;
  function trail(bucket2, prefix) {
    const steps = [
      el("a", { href: format({ kind: "buckets" }) }, "Buckets"),
      icon("chevron")
    ];
    const parts = prefix.split("/").filter((piece) => piece !== "");
    const step = (label, at2, last) => last ? el("span", { "aria-current": "page" }, label) : el("a", { href: format({ kind: "objects", bucket: bucket2, prefix: at2, cursor: null, key: null, filter: "" }) }, label);
    steps.push(step(bucket2, "", parts.length === 0));
    let walked = "";
    parts.forEach((part, index) => {
      walked += `${part}/`;
      steps.push(icon("chevron"), step(part, walked, index === parts.length - 1));
    });
    return el("nav", { class: "trail", "aria-label": "Prefix" }, ...steps);
  }
  async function objects(screen, bucket2, prefix, cursor, selected, filter) {
    const title = bucket2;
    loading(screen, title, "keys");
    const fetchPage = (named2, after) => call(
      "GET",
      `/buckets/${encodeURIComponent(bucket2)}/objects${search([["prefix", `${prefix}${named2}` === "" ? null : `${prefix}${named2}`], ["delimiter", "/"], ["cursor", after], ["limit", PAGE2]])}`,
      readListing
    );
    const answer = await fetchPage(filter, cursor);
    if (!answer.ok) {
      failed(screen, title, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    let named = filter;
    let from = cursor;
    const here = (key) => format({ kind: "objects", bucket: bucket2, prefix, cursor: from, key, filter: named });
    const links = /* @__PURE__ */ new Map();
    const folderRow = (folder) => row(el("a", { class: "name", href: format({ kind: "objects", bucket: bucket2, prefix: folder, cursor: null, key: null, filter: "" }) }, icon("folder"), mono(folder.slice(prefix.length))), "Folder", "", "", "", "");
    const fileRow = (object2) => {
      const kind = kindOf(object2.contentType);
      const link = el("a", { class: "name", href: here(object2.key), "data-key": object2.key }, icon(kind.icon), mono(object2.key.slice(prefix.length)));
      link.addEventListener("click", (event) => {
        if (event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey) {
          event.preventDefault();
          select(object2.key);
        }
      });
      links.set(object2.key, link);
      const buttons = el("div", { class: "actions" });
      const line = row(
        link,
        el("span", { title: object2.contentType ?? "No content type was stored" }, kind.label),
        el("span", { title: `${object2.size.toLocaleString()} bytes` }, size(object2.size)),
        moment(object2.modified),
        // The full ETag is in the details; eight hex digits tell two versions apart at a glance.
        el("span", { title: object2.etag }, mono(object2.etag.length > 12 ? `${object2.etag.slice(0, 9)}…` : object2.etag)),
        buttons
      );
      const forget = () => {
        links.delete(object2.key);
        if (shown === object2.key) {
          select(null);
        }
      };
      buttons.append(...objectActions(screen, bucket2, object2, line, () => select(object2.key), forget));
      return line;
    };
    const rowsOf = (listing) => [...listing.prefixes.map(folderRow), ...listing.objects.map(fileRow)];
    const nothing = () => named !== "" ? empty(`No name in this folder starts with “${named}”.`) : from !== null ? empty("No more keys on this page.", el("a", { href: format({ kind: "objects", bucket: bucket2, prefix, cursor: null, key: null, filter: "" }) }, "Back to the first page")) : prefix === "" ? empty("This bucket is empty.") : empty("Nothing under this prefix.", el("a", { href: format({ kind: "objects", bucket: bucket2, prefix: "", cursor: null, key: null, filter: "" }) }, "Back to the top of the bucket"));
    const box2 = field("name-filter", "Filter by name", { type: "search", value: filter, autocomplete: "off", spellcheck: "false" }, "Names in this folder starting with this text; the server lists only those.");
    const toolbar = el("div", { class: "toolbar" }, box2.row);
    const list2 = el("section", { class: "card flush" }, toolbar);
    let current2 = null;
    const show = (listing) => {
      current2?.stop();
      links.clear();
      const caption = `Keys under ${prefix === "" ? "the bucket" : prefix}${named === "" ? "" : ` starting “${named}”`}`;
      const back = from === null ? null : el("a", { href: format({ kind: "objects", bucket: bucket2, prefix, cursor: null, key: null, filter: named }) }, "Back to the first page");
      current2 = run(screen, listing, rowsOf, (after) => fetchPage(named, after), caption, nothing(), back);
      while (toolbar.nextSibling !== null) {
        toolbar.nextSibling.remove();
      }
      list2.append(...current2.nodes);
      current2.start();
      links.get(shown ?? "")?.closest("tr")?.setAttribute("aria-current", "true");
    };
    let typing;
    let asked = 0;
    box2.input.addEventListener("input", () => {
      window.clearTimeout(typing);
      typing = window.setTimeout(() => {
        void (async () => {
          if (box2.input.value === named) {
            return;
          }
          named = box2.input.value;
          from = null;
          asked += 1;
          const mine = asked;
          history.replaceState(null, "", here(shown));
          const answered = await fetchPage(named, null);
          if (!screen.live() || mine !== asked) {
            return;
          }
          if (answered.ok) {
            show(answered.value);
          } else if (answered.status === 401) {
            screen.signIn();
          } else {
            current2?.stop();
            while (toolbar.nextSibling !== null) {
              toolbar.nextSibling.remove();
            }
            list2.append(el("p", { class: "paging", role: "alert" }, answered.message));
          }
        })();
      }, FILTER_PAUSE_MS);
    });
    const pane = el("aside", { class: "pane card", "aria-labelledby": "pane-title", hidden: "" });
    const split = el("div", { class: "split" }, list2, pane);
    let shown = null;
    let turn = 0;
    const select = (key) => {
      links.get(shown ?? "")?.closest("tr")?.removeAttribute("aria-current");
      shown = key;
      turn += 1;
      history.replaceState(null, "", here(key));
      if (key === null) {
        pane.hidden = true;
        split.classList.remove("open");
        fill(pane);
        return;
      }
      links.get(key)?.closest("tr")?.setAttribute("aria-current", "true");
      pane.hidden = false;
      split.classList.add("open");
      if (window.matchMedia("(max-width: 64rem)").matches) {
        pane.scrollIntoView({ block: "start" });
      }
      const mine = turn;
      void describe(screen, pane, bucket2, key, close, () => mine === turn);
    };
    function close() {
      const was = shown;
      select(null);
      links.get(was ?? "")?.focus();
    }
    split.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && shown !== null && event.target !== box2.input) {
        event.preventDefault();
        close();
        return;
      }
      const target = event.target;
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp" || !(target instanceof HTMLAnchorElement) || !target.classList.contains("name")) {
        return;
      }
      const names2 = Array.from(list2.querySelectorAll("a.name"));
      const next2 = names2[names2.indexOf(target) + (event.key === "ArrowDown" ? 1 : -1)];
      if (next2 === void 0) {
        return;
      }
      event.preventDefault();
      next2.focus();
      const key = next2.dataset["key"];
      if (key !== void 0) {
        select(key);
      }
    });
    show(answer.value);
    fill(screen.main, head(title, trail(bucket2, prefix)), split);
    if (selected !== null) {
      select(selected);
    }
  }

  // src/renditions.ts
  //! Renditions that failed for good: the tasks whose five hand-outs are spent,
  //! with the original, the output, and the last error a worker recorded. They
  //! stay listed until the original is written again; for operators.
  var TITLE6 = "Renditions";
  async function renditions(screen) {
    loading(screen, TITLE6, "failed renditions");
    const answer = await call("GET", "/media/failed", readFailedRenditions);
    if (!answer.ok) {
      failed(screen, TITLE6, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const page = answer.value;
    const rows = page.failed.map(
      (one) => row(
        mono(`${one.bucket}/${one.key}`),
        one.version_id === null ? el("span", { class: "muted" }, "current") : mono(one.version_id),
        mono(one.output),
        el("span", { class: "numeric" }, String(one.attempts)),
        one.error ?? el("span", { class: "muted" }, "none recorded")
      )
    );
    fill(
      screen.main,
      head(TITLE6, "Renditions the workers stopped trying after five attempts, with the last error. Writing the original again plans it anew."),
      el(
        "section",
        { class: "card flush" },
        rows.length === 0 ? empty("No rendition has failed.") : table("Failed renditions", ["Original", "Version", "Output", "Tries", "Last error"], rows),
        page.more ? el("p", { class: "paging" }, "Showing the first 100 of each queue.") : null
      )
    );
  }

  // src/signin-model.ts
  //! What the sign-in page is told before anyone signs in: whether this node offers signing in through an identity
  //! provider, besides signing in with a key.
  function readMethods(v) {
    if (typeof v !== "object" || v === null || !("provider" in v) || !("keys" in v)) {
      return null;
    }
    const provider = v.provider;
    const keys2 = v.keys;
    if (typeof provider !== "boolean" || keys2 !== "all" && keys2 !== "root") {
      return null;
    }
    return { provider, keys: keys2 };
  }
  var PROVIDER_SIGN_IN = "/api/v1/auth/oidc/start?fresh=true";

  // src/signin.ts
  //! Signing in — through the organisation's identity provider when this node offers one, or with an access key: the
  //! node's root key or a key issued to a user. The secret goes to the server once; what the browser keeps is an
  //! HttpOnly session cookie it cannot read, and the fields are cleared as soon as the answer arrives. Beside a
  //! provider the root key is the emergency path: the server asks for a reason, and the form then shows that field.
  async function providerChoice(keyForm) {
    const methods = await call("GET", "/auth/methods", readMethods);
    if (!methods.ok || !methods.value.provider) {
      return null;
    }
    const provider = el("a", { class: "button primary provider", href: PROVIDER_SIGN_IN }, "Sign in with your organisation");
    if (methods.value.keys === "all") {
      return {
        choice: el("div", {}, provider, el("p", { class: "muted" }, "or with an access key")),
        intro: "Sign in through your organisation. An access key also works: one issued to your user, or this node's root key in an emergency."
      };
    }
    for (const part of keyForm) {
      part.hidden = true;
    }
    const emergency = el("button", { type: "button", class: "link" }, "Emergency sign-in with the root key");
    emergency.addEventListener("click", () => {
      for (const part of keyForm) {
        part.hidden = false;
      }
      emergency.hidden = true;
      keyForm[0]?.querySelector("input")?.focus();
    });
    return {
      choice: el("div", {}, provider, el("p", { class: "muted" }, emergency)),
      intro: "Sign in through your organisation. Access keys do not open a session here, except this node's root key in an emergency."
    };
  }
  function signIn(main2, done5) {
    const key = field("access-key", "Access key ID", { autocomplete: "username", spellcheck: "false", required: "" });
    const secret = field("secret-key", "Secret access key", { type: "password", autocomplete: "current-password", required: "" });
    const reason = field("reason", "Reason", { autocomplete: "off" }, "Why the root key, now: it is recorded and announced.");
    reason.row.hidden = true;
    const submit = el("button", { type: "submit", class: "primary" }, "Sign in");
    const said = el("div", { "aria-live": "polite" });
    const choice = el("div", {});
    const intro = el("p", { class: "muted" }, "Use your access key: one issued to your user, or the root key this node was started with. The session lasts one hour.");
    const form = el(
      "form",
      { class: "card", novalidate: "" },
      el("img", { class: "logo", src: "/favicon.svg", alt: "", width: "48", height: "48" }),
      heading("Sign in to TessariDB S3"),
      intro,
      choice,
      key.row,
      secret.row,
      reason.row,
      submit,
      said
    );
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      submit.disabled = true;
      const why = reason.input.value.trim();
      const answer = await call("POST", "/session", ignored, {
        access_key_id: key.input.value.trim(),
        secret_access_key: secret.input.value,
        ...why === "" ? {} : { reason: why }
      });
      secret.input.value = "";
      submit.disabled = false;
      if (answer.ok) {
        key.input.value = "";
        reason.input.value = "";
        done5();
        return;
      }
      if (answer.code === "reason_required") {
        reason.row.hidden = false;
        fill(said, refusal({ ...answer, message: "This node signs people in through its identity provider; the root key is its emergency path. Say why, then sign in again: the session lasts 15 minutes and is recorded and announced." }));
        reason.input.focus();
        return;
      }
      const failure = answer.status === 401 ? { ...answer, message: "That access key and secret do not match this node's root credential." } : answer;
      fill(said, refusal(failure));
      secret.input.focus();
    });
    fill(main2, el("div", { class: "gate" }, form));
    key.input.focus();
    void providerChoice([key.row, secret.row, submit]).then((offered) => {
      if (offered !== null) {
        fill(intro, offered.intro);
        submit.classList.remove("primary");
        fill(choice, offered.choice);
      }
    });
  }

  // src/spaces.ts
  //! Spaces: the tenants buckets and users belong to. Only operators reach this
  //! view; anyone else is told their key does not allow it. A new space takes a
  //! reason, which goes into the action record.
  var TITLE7 = "Spaces";
  function createForm3(screen, opener) {
    const name = field("new-space", "Space name", { spellcheck: "false", autocomplete: "off", required: "" }, "1-63 lowercase letters, digits and inner hyphens.");
    const reason = field("new-space-reason", "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
    const said = el("div", { "aria-live": "polite" });
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    const form = el(
      "form",
      { id: "new-space-form", class: "card", novalidate: "", hidden: "", "aria-labelledby": "new-space-title" },
      el("h2", { id: "new-space-title" }, "New space"),
      name.row,
      reason.row,
      el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Create space"), cancel),
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
      const answer = await call("POST", "/spaces", ignored, { name: wanted, reason: reason.input.value.trim() });
      if (!answer.ok) {
        if (answer.status === 401) {
          screen.signIn();
          return;
        }
        fill(said, refusal(answer));
        return;
      }
      announce(`Created space ${wanted}.`);
      screen.redraw();
    });
    return form;
  }
  async function spaces(screen) {
    loading(screen, TITLE7, "spaces");
    const answer = await call("GET", "/spaces", readSpaces);
    if (!answer.ok) {
      failed(screen, TITLE7, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const opener = el("button", { type: "button", class: "primary", "aria-expanded": "false", "aria-controls": "new-space-form" }, icon("plus"), "New space");
    fill(
      screen.main,
      head(TITLE7, `${answer.value.length.toLocaleString()} on this store`, opener),
      createForm3(screen, opener),
      el(
        "section",
        { class: "card flush" },
        answer.value.length === 0 ? empty("No spaces yet.") : table("Spaces", ["Name", "Created"], answer.value.map((space2) => row(mono(space2.name), moment(space2.created))))
      )
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

  // src/users.ts
  //! Users: operators see every space's users, a space admin its own space's.
  //! Creating a user is judged by the server on the user it would be, so a space
  //! admin who asks for an administrator, an operator or a cluster viewer is told
  //! the key does not allow it. Each row's actions are in `user-actions.ts`.
  var TITLE8 = "Users";
  function check(id, label) {
    const input = el("input", { id, name: id, type: "checkbox" });
    return { row: el("label", { class: "check", for: id }, input, label), input };
  }
  function createForm4(screen, opener) {
    const name = field("new-user", "User name", { spellcheck: "false", autocomplete: "off", required: "" }, "1-63 lowercase letters, digits and inner hyphens; unique across the store.");
    const space2 = field("new-user-space", "Space", { spellcheck: "false", autocomplete: "off", required: "" });
    const role2 = el("select", { id: "new-user-role" }, el("option", { value: "member" }, "Member"), el("option", { value: "space_admin" }, "Space admin"));
    const creates = check("new-user-creates", "May create buckets");
    const operates = check("new-user-operator", "Operator (every space)");
    const views = check("new-user-viewer", "Sees the cluster");
    const reason = field("new-user-reason", "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
    const said = el("div", { "aria-live": "polite" });
    const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
    const form = el(
      "form",
      { id: "new-user-form", class: "card", novalidate: "", hidden: "", "aria-labelledby": "new-user-title" },
      el("h2", { id: "new-user-title" }, "New user"),
      name.row,
      space2.row,
      el("div", { class: "field" }, el("label", { for: role2.id }, "Role"), role2),
      el("fieldset", { class: "checks" }, el("legend", {}, "Permissions"), creates.row, operates.row, views.row),
      reason.row,
      el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Create user"), cancel),
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
      const answer = await call("POST", "/users", ignored, {
        name: wanted,
        space: space2.input.value.trim(),
        role: role2.value,
        create_buckets: creates.input.checked,
        operator: operates.input.checked,
        cluster_viewer: views.input.checked,
        reason: reason.input.value.trim()
      });
      if (!answer.ok) {
        if (answer.status === 401) {
          screen.signIn();
          return;
        }
        fill(said, refusal(answer));
        return;
      }
      announce(`Created user ${wanted}. Issue a key so it can sign in.`);
      screen.redraw();
    });
    return form;
  }
  function standing(user2) {
    const parts = [user2.role === "space_admin" ? "space admin" : "member"];
    if (user2.create_buckets) parts.push("creates buckets");
    if (user2.operator) parts.push("operator");
    if (user2.cluster_viewer) parts.push("sees the cluster");
    return parts.join(" · ");
  }
  function userRow(screen, user2) {
    const state2 = el("span", { class: user2.disabled ? "chip bad" : "chip ok" }, user2.disabled ? "Disabled" : "Active");
    const buttons = el("div", { class: "actions" });
    const line = row(mono(user2.name), mono(user2.space), standing(user2), state2, moment(user2.created), buttons);
    buttons.append(...rowActions(screen, user2, line));
    return line;
  }
  async function users(screen) {
    loading(screen, TITLE8, "users");
    const answer = await call("GET", "/users", readUsers);
    if (!answer.ok) {
      failed(screen, TITLE8, answer);
      return;
    }
    if (!screen.live()) {
      return;
    }
    const opener = el("button", { type: "button", class: "primary", "aria-expanded": "false", "aria-controls": "new-user-form" }, icon("plus"), "New user");
    fill(
      screen.main,
      head(TITLE8, `${answer.value.length.toLocaleString()} you administer`, opener),
      createForm4(screen, opener),
      el(
        "section",
        { class: "card flush" },
        answer.value.length === 0 ? empty("No users yet. Create one with “New user”, then issue it a key.") : table("Users", ["Name", "Space", "Standing", "State", "Created", "Actions"], answer.value.map((user2) => userRow(screen, user2)))
      )
    );
  }

  // src/console.ts
  //! The console's entry point: one view at a time, chosen by the URL's hash, so
  //! a reload or a shared link lands on the same view. A view that answers 401
  //! hands over to signing in, and signing in draws the same view again.
  var main = at("view");
  var signOut = at("sign-out");
  var generation = 0;
  var may = null;
  var NEEDS = {
    users: (can) => can.administer,
    spaces: (can) => can.operate,
    cluster: (can) => can.view_cluster,
    renditions: (can) => can.operate,
    actions: (can) => can.operate
  };
  function offer(can) {
    for (const link of all("[data-section]")) {
      const need = NEEDS[link.dataset["section"] ?? ""];
      link.hidden = need !== void 0 && !need(can);
    }
  }
  function draw(route, screen) {
    switch (route.kind) {
      case "status":
        return status(screen);
      case "buckets":
        return buckets(screen);
      case "objects":
        return objects(screen, route.bucket, route.prefix, route.cursor, route.key, route.filter);
      case "object":
        return object(screen, route.bucket, route.key);
      case "users":
        return users(screen);
      case "spaces":
        return spaces(screen);
      case "cluster":
        return cluster(screen);
      case "renditions":
        return renditions(screen);
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
    may = null;
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
    if (may === null) {
      const asked = await call("GET", "/session", readCapabilities);
      if (mine !== generation) {
        return;
      }
      if (!asked.ok && asked.status === 401) {
        showSignIn();
        return;
      }
      if (asked.ok) {
        may = asked.value;
        offer(may);
      }
    }
    const route = parse(location.hash);
    mark(route);
    const screen = {
      main,
      live: () => mine === generation,
      signIn: showSignIn,
      redraw: () => void render(),
      may
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
  var SECTION_ICONS = { status: "overview", buckets: "buckets", users: "users", spaces: "layers", cluster: "members", renditions: "image", actions: "record" };
  for (const link of all("[data-section]")) {
    const name = SECTION_ICONS[link.dataset["section"] ?? ""];
    if (name !== void 0) {
      link.prepend(icon(name));
    }
  }
  signOut.prepend(icon("sign-out"));
  installTips();
  themes(at("theme"));
  at("skip").addEventListener("click", (event) => {
    event.preventDefault();
    main.focus();
  });
  window.addEventListener("hashchange", () => void render());
  void render();
})();
