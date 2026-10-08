//! The console's entry point: one view at a time, chosen by the URL's hash, so
//! a reload or a shared link lands on the same view. A view that answers 401
//! hands over to signing in, and signing in draws the same view again.

import { actions } from "./actions.ts";
import { call } from "./api.ts";
import { buckets } from "./buckets.ts";
import { cluster } from "./cluster.ts";
import { all, announce, at } from "./dom.ts";
import { ignored, readCapabilities, type Capabilities } from "./models.ts";
import { object } from "./object.ts";
import { objects } from "./objects.ts";
import { parse, type Route } from "./route.ts";
import type { Screen } from "./screen.ts";
import { signIn } from "./signin.ts";
import { icon, type IconName } from "./icons.ts";
import { spaces } from "./spaces.ts";
import { status } from "./status.ts";
import { themes } from "./theme.ts";
import { users } from "./users.ts";

const main = at("view");
const signOut = at("sign-out");

/** Bumped by every render, so an answer arriving for an older view paints nothing. */
let generation = 0;

/** What the signed-in key may do; asked once per sign-in, forgotten on signing out. */
let may: Capabilities | null = null;

/** The capability each gated section needs; a section not listed is open to every signed-in key. */
const NEEDS: Readonly<Record<string, (can: Capabilities) => boolean>> = {
  users: (can) => can.administer,
  spaces: (can) => can.operate,
  cluster: (can) => can.view_cluster,
  actions: (can) => can.operate,
};

/** Shows only the sections the key may use. The server still refuses the others; this only stops offering them. */
function offer(can: Capabilities): void {
  for (const link of all("[data-section]")) {
    const need = NEEDS[link.dataset["section"] ?? ""];
    link.hidden = need !== undefined && !need(can);
  }
}

function draw(route: Route, screen: Screen): Promise<void> {
  switch (route.kind) {
    case "status":
      return status(screen);
    case "buckets":
      return buckets(screen);
    case "objects":
      return objects(screen, route.bucket, route.prefix, route.cursor);
    case "object":
      return object(screen, route.bucket, route.key);
    case "users":
      return users(screen);
    case "spaces":
      return spaces(screen);
    case "cluster":
      return cluster(screen);
    case "actions":
      return actions(screen, route.before);
  }
}

/** Marks the navigation link for the section `route` belongs to. */
function mark(route: Route): void {
  const section = route.kind === "objects" || route.kind === "object" ? "buckets" : route.kind;
  for (const link of all("[data-section]")) {
    if (link.dataset["section"] === section) {
      link.setAttribute("aria-current", "page");
    } else {
      link.removeAttribute("aria-current");
    }
  }
}

function showSignIn(): void {
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

async function render(): Promise<void> {
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
  const screen: Screen = {
    main,
    live: () => mine === generation,
    signIn: showSignIn,
    redraw: () => void render(),
  };
  await draw(route, screen);
  if (screen.live()) {
    main.querySelector<HTMLElement>(".view-title")?.focus({ preventScroll: true });
  }
}

signOut.addEventListener("click", async () => {
  await call("DELETE", "/session", ignored);
  announce("Signed out.");
  showSignIn();
});
const SECTION_ICONS: Readonly<Record<string, IconName>> = { status: "overview", buckets: "buckets", users: "users", spaces: "layers", cluster: "members", actions: "record" };
for (const link of all("[data-section]")) {
  const name = SECTION_ICONS[link.dataset["section"] ?? ""];
  if (name !== undefined) {
    link.prepend(icon(name));
  }
}
signOut.prepend(icon("sign-out"));
themes(at("theme"));

// The skip link must move focus, not the hash: the hash is the route.
at("skip").addEventListener("click", (event) => {
  event.preventDefault();
  main.focus();
});
window.addEventListener("hashchange", () => void render());
void render();
