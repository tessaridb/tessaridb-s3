//! The overview: is this node and its cluster healthy, and is anything waiting
//! to be healed. Each tile links to where the operator acts on it.

import { call } from "./api.ts";
import { el, fill, mono, row, table } from "./dom.ts";
import { icon, type IconName } from "./icons.ts";
import { readStatus, type Backlog, type Member } from "./models.ts";
import { format } from "./route.ts";
import { empty, failed, head, loading, type Screen } from "./screen.ts";

const TITLE = "Overview";

/** A figure with its label and a line of context; a link when `href` is given. */
function tile(glyph: IconName, label: string, value: Node | string, note: Node | string, href?: string): HTMLElement {
  const parts = [el("span", { class: "label" }, icon(glyph), label), el("span", { class: "value" }, value), el("span", { class: "note" }, note)];
  return href === undefined ? el("div", { class: "tile" }, ...parts) : el("a", { class: "tile", href }, ...parts);
}

function healing(heal: Backlog | null): HTMLElement {
  if (heal === null) {
    return tile("pulse", "Healing", "—", "Applies to cluster members; this node stores objects on its own.");
  }
  if (heal.listed === 0) {
    return tile("pulse", "Healing", el("span", { class: "chip ok" }, "Healthy"), "No objects are waiting to be healed.");
  }
  const counted = heal.more ? `> ${heal.listed.toLocaleString()}` : heal.listed.toLocaleString();
  return tile("pulse", "Healing", counted, el("span", {}, el("span", { class: "chip warn" }, "Waiting"), " Members heal these objects in the background."));
}

/** Whether a member answered just now, as text and colour. */
function state(member: Member): HTMLElement {
  return member.answering ? el("span", { class: "chip ok" }, "Answering") : el("span", { class: "chip bad" }, "Not answering");
}

/** The Members tile: how many are registered and, when any is silent, how many answer. */
function membersTile(members: readonly Member[] | null): HTMLElement {
  if (members === null) {
    return tile("members", "Members", "—", "No cluster");
  }
  const answering = members.filter((member) => member.answering).length;
  return answering === members.length
    ? tile("members", "Members", String(members.length), "Registered and answering")
    : tile("members", "Members", String(members.length), el("span", { class: "chip bad" }, `${answering} of ${members.length} answering`));
}

export async function status(screen: Screen): Promise<void> {
  loading(screen, TITLE, "the node's status");
  const answer = await call("GET", "/status", readStatus);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const node = answer.value;
  const members =
    node.members === null
      ? null
      : node.members.length === 0
        ? empty("No members are registered yet.")
        : table("Cluster members", ["Member", "State", "Internal address"], node.members.map((member) => row(mono(member.node), state(member), mono(member.endpoint))));
  fill(
    screen.main,
    head(TITLE, el("span", {}, "Region ", mono(node.region), " · version ", mono(node.version))),
    el(
      "div",
      { class: "tiles" },
      tile("node", "Node", node.node === null ? "Single" : mono(node.node), node.node === null ? "Not a cluster member" : "This node's name in the cluster"),
      membersTile(node.members),
      tile("layers", "Erasure code", node.erasure === null ? "None" : mono(node.erasure), node.erasure === null ? "Whole objects on one node" : "Data + parity shards per object"),
      healing(node.heal_backlog),
      tile("buckets", "Buckets", "Browse", "Find a bucket or an object", format({ kind: "buckets" })),
      tile("record", "Action record", "Review", "Every console change and download", format({ kind: "actions", before: null })),
    ),
    members === null ? null : el("section", { class: "card flush" }, el("h2", {}, "Members"), members),
  );
}
