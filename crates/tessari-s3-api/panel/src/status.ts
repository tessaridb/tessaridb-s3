//! The overview: is this node and its cluster healthy, how much is stored and
//! how full the drives are, and is anything waiting to be healed. Each tile
//! links to where the operator acts on it.

import { call, type Answer } from "./api.ts";
import { el, fill, mono, row, table } from "./dom.ts";
import { amount, moment, share, size } from "./format.ts";
import { icon, type IconName } from "./icons.ts";
import { meter, used } from "./meter.ts";
import { readStatus, readUsage, type Backlog, type Drive, type Member, type Status, type Usage } from "./models.ts";
import { format } from "./route.ts";
import { empty, failed, head, loading, type Screen } from "./screen.ts";

const TITLE = "Overview";

/** A figure with its label and a line of context; a link when `href` is given. */
export function tile(glyph: IconName, label: string, value: Node | string, note: Node | string, href?: string): HTMLElement {
  const parts = [el("span", { class: "label" }, icon(glyph), label), el("span", { class: "value" }, value), el("span", { class: "note" }, note)];
  return href === undefined ? el("div", { class: "tile" }, ...parts) : el("a", { class: "tile", href }, ...parts);
}

export function healing(heal: Backlog | null): HTMLElement {
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

/** The members with whether each answers, its disk and its internal address. */
export function memberTable(members: readonly Member[]): HTMLElement {
  return members.length === 0
    ? empty("No members are registered yet.")
    : table(
        "Cluster members",
        ["Member", "State", "Disk", "Internal address"],
        members.map((member) =>
          row(mono(member.node), state(member), member.drive === null ? "—" : meter(member.drive, `Disk space used on ${member.node}`), mono(member.endpoint)),
        ),
      );
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

/** The Stored tile: what clients stored, as of the last measurement. */
function storedTile(usage: Answer<Usage>): HTMLElement {
  if (!usage.ok) {
    return tile("buckets", "Stored", "—", "The usage figures could not be read. Reload to try again.");
  }
  const measured = usage.value;
  if (measured.taken === null) {
    return tile("buckets", "Stored", "—", "Not measured yet. A node measures every minute after it starts.");
  }
  const objects = measured.objects === 1 ? "1 object" : `${amount(measured.objects)} objects`;
  return tile(
    "buckets",
    "Stored",
    size(measured.bytes),
    `${objects} · ${size(measured.raw_bytes)} on the drives, ${size(measured.inline_bytes)} in the metadata · measured ${moment(measured.taken)}`,
  );
}

/** Every drive the console could read: this node's off a cluster, each answering member's on one. */
function drives(node: Status): Drive[] {
  if (node.members === null) {
    return node.drive === null ? [] : [node.drive];
  }
  return node.members.flatMap((member) => (member.drive === null ? [] : [member.drive]));
}

/** The Disk tile: space used over every drive that reported, with the bar. */
function diskTile(node: Status): HTMLElement {
  const read = drives(node);
  if (read.length === 0) {
    return tile("disk", "Disk", "—", node.members === null ? "This node stores no data on its own drive." : "No member reported its drive.");
  }
  const total: Drive = read.reduce(
    (sum, drive) => ({ capacity: sum.capacity + drive.capacity, free: sum.free + drive.free, available: sum.available + drive.available }),
    { capacity: 0, free: 0, available: 0 },
  );
  const scope = node.members === null ? "This node's data drive" : `Raw space on ${read.length} of ${node.members.length} members`;
  return tile(
    "disk",
    "Disk",
    el("span", {}, share(used(total), total.capacity), el("span", { class: "muted" }, " used")),
    el("span", { class: "gauge" }, meter(total, "Disk space used"), el("span", {}, `${scope} · ${size(total.free)} free`)),
  );
}

export async function status(screen: Screen): Promise<void> {
  loading(screen, TITLE, "the node's status");
  const [answer, usage] = await Promise.all([call("GET", "/status", readStatus), call("GET", "/usage", readUsage)]);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const node = answer.value;
  const members = node.members === null ? null : memberTable(node.members);
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
      storedTile(usage),
      diskTile(node),
      tile("buckets", "Buckets", "Browse", "Find a bucket or an object", format({ kind: "buckets" })),
      tile("record", "Action record", "Review", "Every console change and download", format({ kind: "actions", before: null })),
    ),
    members === null ? null : el("section", { class: "card flush" }, el("h2", {}, "Members"), members),
  );
}
