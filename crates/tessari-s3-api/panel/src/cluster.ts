//! The cluster: its members with whether each answers and its disk, the
//! layout objects are placed by and its erasure code, the healing backlog,
//! and the metadata nodes. Only a key that may see the cluster is shown this
//! view; the server refuses everyone else.

import { call } from "./api.ts";
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
  return el(
    "section",
    { class: "card" },
    el("h2", {}, "Layout"),
    el("p", { class: "muted" }, `Version ${cluster.layout.version}. Shard 1 of every object goes to the first node, shard 2 to the second, and so on.`),
    el("ol", {}, ...cluster.layout.nodes.map((node) => el("li", {}, mono(node)))),
  );
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
  fill(
    screen.main,
    head(TITLE, view.node === null ? "A node on its own" : el("span", {}, "This node: ", mono(view.node))),
    el(
      "div",
      { class: "tiles" },
      tile("layers", "Erasure code", view.erasure === null ? "—" : mono(view.erasure), view.erasure === null ? "Not a cluster member" : "data + parity shards per object"),
      healing(view.erasure === null ? null : view.heal_backlog),
      tile("node", "Metadata nodes", String(view.metadata.addresses.length), el("span", {}, ...view.metadata.addresses.flatMap((address, i) => [i === 0 ? "" : ", ", mono(address)]))),
    ),
    view.members === null ? null : el("section", { class: "card flush" }, el("h2", {}, "Members"), memberTable(view.members)),
    layoutCard(view),
  );
}
