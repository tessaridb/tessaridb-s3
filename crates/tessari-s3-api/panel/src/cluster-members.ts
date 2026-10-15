//! Cluster admission: every node an operator admitted or removed, with its
//! pinned certificate and whether a join token waits; and, for an operator, the
//! forms that admit a node, issue a one-time join token (shown once, here, and
//! nowhere else) and remove a node for good. The server refuses everyone else
//! and wants a reason and a recent sign-in for every change.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, field, fill, mono, row, table } from "./dom.ts";
import { moment } from "./format.ts";
import { isFingerprint, readMember, readMembers, readToken, type Member } from "./members-model.ts";
import { ignored } from "./models.ts";
import { empty, refusal, type Failure, type Screen } from "./screen.ts";

const BASE = "/cluster/members";
/** Token lifetimes offered: ten minutes, an hour, a day (the server takes 60 s to one day). */
const LIFETIMES: ReadonlyArray<readonly [string, number]> = [
  ["10 minutes", 600],
  ["1 hour", 3600],
  ["1 day", 86400],
];

/** After a change: success redraws, a 401 goes to sign-in, a refusal is returned to show. */
function done(screen: Screen, failure: Failure | null, said: string): Failure | null {
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

function stateChip(member: Member): HTMLElement {
  if (member.state === "removed") {
    return el("span", { class: "chip bad" }, "Removed");
  }
  return member.awaiting_token ? el("span", { class: "chip warn" }, "Awaiting token") : el("span", { class: "chip ok" }, "Admitted");
}

function certificate(member: Member): Node | string {
  if (member.fingerprint !== null) {
    return el("code", { title: member.fingerprint }, `${member.fingerprint.slice(0, 16)}…`);
  }
  if (member.state === "removed") {
    return "—";
  }
  return member.awaiting_token && member.token_expires !== null ? `Token lapses ${moment(member.token_expires)}` : "Any the cluster issued for it";
}

function removeButton(screen: Screen, member: Member, region: HTMLElement): HTMLElement {
  const button = el("button", { type: "button", class: "danger" }, "Remove…");
  button.addEventListener("click", () =>
    fill(
      region,
      confirmation(
        `member-remove-${member.node}`,
        mono(member.node),
        "The node is removed for good: it is never admitted again and its registration is deleted. Refused while the current layout places shards on it — apply a layout without it first.",
        "Remove",
        async (reason) => {
          const answer = await call("POST", `${BASE}/remove`, ignored, { node: member.node, reason });
          return done(screen, answer.ok ? null : answer, `Removed ${member.node}.`);
        },
        () => fill(region),
      ),
    ),
  );
  return button;
}

function memberRows(screen: Screen, members: readonly Member[], region: HTMLElement): HTMLElement {
  if (members.length === 0) {
    return empty("No node has been admitted yet.");
  }
  const operate = screen.may?.operate === true;
  const head = ["Node", "State", "Certificate", "Reason", "By", "When", ...(operate ? [""] : [])];
  return table(
    "Cluster admission",
    head,
    members.map((member) =>
      row(
        mono(member.node),
        stateChip(member),
        certificate(member),
        member.reason,
        member.by,
        moment(member.at),
        ...(operate ? [member.state === "approved" ? removeButton(screen, member, region) : ""] : []),
      ),
    ),
  );
}

function reasonField(id: string): HTMLTextAreaElement {
  return el("textarea", { id, rows: "2", maxlength: "500", required: "" });
}

function admitForm(screen: Screen): HTMLElement {
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
    said,
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

/** The join token, shown this once, with a copy button. */
function shownOnce(node: string, token: string, expires: string, done: () => void): HTMLElement {
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
  finish.addEventListener("click", done);
  queueMicrotask(() => copy.focus());
  return el(
    "div",
    { class: "once", role: "alert" },
    el("p", {}, el("strong", {}, "Store this token now. "), "It is shown once; the server keeps only its digest."),
    el("dl", {}, el("dt", {}, "Node"), el("dd", {}, mono(node)), el("dt", {}, "Lapses"), el("dd", {}, moment(expires)), el("dt", {}, "Join token"), el("dd", {}, mono(token))),
    el("div", { class: "actions" }, copy, finish),
    said,
  );
}

function tokenForm(screen: Screen): HTMLElement {
  const node = field("token-node", "Node id", { autocomplete: "off", spellcheck: "false", required: "" });
  const lifetime = el("select", { id: "token-lifetime" }, ...LIFETIMES.map(([label, secs]) => el("option", { value: String(secs) }, label)));
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
    said,
  );
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const body = { node: node.input.value.trim(), lifetime_secs: Number(lifetime.value), reason: reason.value.trim() };
    const answer = await call("POST", `${BASE}/token`, readToken, body);
    if (!answer.ok) {
      fill(said, refusal(answer));
      return;
    }
    // Shown here once; "I have stored it" redraws the page, which takes the token out of it.
    const { token, expires } = answer.value;
    announce(`Issued a join token for ${body.node}. Store it now.`);
    fill(said, shownOnce(body.node, token, expires, () => screen.redraw()));
  });
  return form;
}

/** The admission card for a cluster member. */
export async function admissionCard(screen: Screen): Promise<HTMLElement> {
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
