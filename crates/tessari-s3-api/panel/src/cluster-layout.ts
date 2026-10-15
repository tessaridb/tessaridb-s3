//! Changing the layout: the staged layout, if any, with Apply and Discard, and
//! the form that stages one — one erasure set per line. Applying names the
//! staging this page showed, so one staged again meanwhile is refused rather
//! than applied unseen. Only an operator is offered the controls; the server
//! refuses everyone else and wants a reason and a recent sign-in either way.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, fill, mono } from "./dom.ts";
import { parseSets, readLayout, readStaged, type Staged } from "./layout-model.ts";
import { ignored, type Layout } from "./models.ts";
import { refusal, type Failure, type Screen } from "./screen.ts";

const STAGED = "/cluster/layout/staged";

function sets(layout: Layout): HTMLElement[] {
  return layout.sets.map((set, i) =>
    el("div", {}, el("h3", {}, `Erasure set ${i + 1}`), el("ol", {}, ...set.map((member) => el("li", {}, mono(member.node), member.zone === null ? "" : el("span", { class: "muted" }, ` — zone ${member.zone}`))))),
  );
}

/** After a change: success redraws, a 401 goes to sign-in (which explains a stale one), a refusal is returned to show. */
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

function stagedPart(screen: Screen, staged: Staged): HTMLElement {
  const { id, base, layout } = staged;
  const region = el("div", {});
  const apply = el("button", { type: "button", class: "primary" }, "Apply…");
  const discard = el("button", { type: "button" }, "Discard…");
  const close = (): void => fill(region);
  apply.addEventListener("click", () =>
    fill(
      region,
      confirmation(
        "layout-apply",
        el("span", {}, `version ${layout.version}, ${layout.sets.length} erasure set${layout.sets.length === 1 ? "" : "s"}`),
        "New objects are written under this layout from now on; objects already written keep the layout they were written under. Refused if the layout or this staging changed since the page showed it.",
        "Apply",
        async (reason) => {
          const answer = await call("POST", "/cluster/layout/apply", readLayout, { id, reason });
          return done(screen, answer.ok ? null : answer, `Applied layout version ${layout.version}.`);
        },
        close,
      ),
    ),
  );
  discard.addEventListener("click", () =>
    fill(
      region,
      confirmation(
        "layout-discard",
        el("span", {}, `the layout staged over version ${base}`),
        "The staged layout is removed; the cluster does not change.",
        "Discard",
        async (reason) => {
          const answer = await call("DELETE", STAGED, ignored, { reason });
          return done(screen, answer.ok ? null : answer, "Discarded the staged layout.");
        },
        close,
      ),
    ),
  );
  return el(
    "div",
    {},
    el("p", { class: "muted" }, `Staged over version ${base}; applying it makes it version ${layout.version}.`),
    ...sets(layout),
    screen.may?.operate === true ? el("div", { class: "actions" }, apply, discard) : null,
    region,
  );
}

function stageForm(screen: Screen): HTMLElement {
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
    said,
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
    const failure = done(screen, answer.ok ? null : answer, "Staged a layout.");
    if (failure !== null) {
      fill(said, refusal(failure));
    }
  });
  return form;
}

/** The card for a cluster member that has a layout: what is staged, and the controls an operator may use. */
export async function stagingCard(screen: Screen): Promise<HTMLElement> {
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
