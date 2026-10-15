//! The cluster: its members with whether each answers and its disk, the
//! layout objects are placed by and its erasure code, the healing backlog,
//! the metadata nodes, which nodes are admitted, which regions are failed over, and a layout change staged or to stage. Only a key that
//! may see the cluster is shown this view; the server refuses everyone else.

import { call } from "./api.ts";
import { admissionCard } from "./cluster-members.ts";
import { stagingCard } from "./cluster-layout.ts";
import { regionsCard } from "./cluster-regions.ts";
import { el, fill, mono } from "./dom.ts";
import { readCluster, type Cluster } from "./models.ts";
import { failed, head, loading, type Screen } from "./screen.ts";
import { healing, memberTable, tile } from "./status.ts";

const TITLE = "Cluster";

function layoutCard(cluster: Cluster): HTMLElement {
  if (cluster.layout === null) {
    const why = cluster.erasure === null ? "This node stores objects on its own." : "The layout is fixed once enough members have registered.";
    return el("section", { class: "card" }, el("h2", {}, "Layout"), el("p", { class: "muted" }, why));
  }
  const { version, sets } = cluster.layout;
  return el(
    "section",
    { class: "card" },
    el("h2", {}, "Layout"),
    el(
      "p",
      { class: "muted" },
      `Version ${version}. Each object goes to one erasure set, chosen from its data id, and each node of that set holds one of its shards; no set keeps more shards in one zone than it can lose.`,
    ),
    ...sets.map((set, i) =>
      el("div", {}, el("h3", {}, sets.length === 1 ? "Erasure set" : `Erasure set ${i + 1}`), el("ol", {}, ...set.map((member) => el("li", {}, mono(member.node), member.zone === null ? "" : el("span", { class: "muted" }, ` — zone ${member.zone}`))))),
    ),
  );
}

/** The objects members still have to move onto the current layout. */
function moving(waiting: Cluster["moving"]): HTMLElement {
  if (waiting.listed === 0) {
    return tile("layers", "Layout moves", el("span", { class: "chip ok" }, "Up to date"), "Every object is on the current layout.");
  }
  const counted = waiting.more ? `> ${waiting.listed.toLocaleString()}` : waiting.listed.toLocaleString();
  return tile("layers", "Layout moves", counted, el("span", {}, el("span", { class: "chip warn" }, "Moving"), " Members move these objects onto the current layout in the background."));
}

export async function cluster(screen: Screen): Promise<void> {
  loading(screen, TITLE, "the cluster");
  const answer = await call("GET", "/cluster", readCluster);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
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
    head(TITLE, view.node === null ? "A node on its own" : el("span", {}, "This node: ", mono(view.node))),
    el(
      "div",
      { class: "tiles" },
      tile("layers", "Erasure code", view.erasure === null ? "—" : mono(view.erasure), view.erasure === null ? "Not a cluster member" : "data + parity shards per object"),
      healing(view.erasure === null ? null : view.heal_backlog),
      view.erasure === null ? null : moving(view.moving),
      tile("node", "Metadata nodes", String(view.metadata.addresses.length), el("span", {}, ...view.metadata.addresses.flatMap((address, i) => [i === 0 ? "" : ", ", mono(address)]))),
    ),
    view.members === null ? null : el("section", { class: "card flush" }, el("h2", {}, "Members"), memberTable(view.members)),
    admission,
    regions,
    layoutCard(view),
    staging,
  );
}
