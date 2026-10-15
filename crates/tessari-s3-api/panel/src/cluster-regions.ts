//! Region failover: every region an operator failed over, under which epoch and
//! to where; and, for an operator, failing a region over to another and
//! restoring it. A failed-over region refuses client writes and still takes the
//! replicas that catch it up. The server wants a reason and a recent sign-in for
//! every change, and records each attempt.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, field, fill, mono, row, table } from "./dom.ts";
import { moment } from "./format.ts";
import { ignored } from "./models.ts";
import { isRegionName, readRegions, type Region } from "./regions-model.ts";
import { empty, refusal, type Failure, type Screen } from "./screen.ts";

const BASE = "/regions";

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

function stateChip(region: Region): HTMLElement {
  return region.failed_over ? el("span", { class: "chip bad" }, "Failed over") : el("span", { class: "chip ok" }, "Taking writes");
}

function restoreButton(screen: Screen, region: Region, area: HTMLElement): HTMLElement {
  const button = el("button", { type: "button" }, "Restore…");
  button.addEventListener("click", () =>
    fill(
      area,
      confirmation(
        `region-restore-${region.region}`,
        mono(region.region),
        "The region takes client writes again, under the next epoch. Restore it once it has caught up from the promoted region and clients may write here again.",
        "Restore",
        async (reason) => {
          const answer = await call("POST", `${BASE}/restore`, ignored, { region: region.region, reason });
          return done(screen, answer.ok ? null : answer, `Restored ${region.region}.`);
        },
        () => fill(area),
      ),
    ),
  );
  return button;
}

function regionRows(screen: Screen, regions: readonly Region[], area: HTMLElement): HTMLElement {
  if (regions.length === 0) {
    return empty("No region has been failed over.");
  }
  const operate = screen.may?.operate === true;
  const head = ["Region", "State", "Writes go to", "Epoch", "Reason", "By", "When", ...(operate ? [""] : [])];
  return table(
    "Region failover",
    head,
    regions.map((region) =>
      row(
        mono(region.region),
        stateChip(region),
        region.promoted === null ? "—" : mono(region.promoted),
        String(region.epoch),
        region.reason,
        region.by,
        moment(region.at),
        ...(operate ? [region.failed_over ? restoreButton(screen, region, area) : ""] : []),
      ),
    ),
  );
}

function failOverForm(screen: Screen, area: HTMLElement): HTMLElement {
  const failed = field("failover-region", "Region to fail over", { autocomplete: "off", spellcheck: "false", required: "" });
  const promoted = field("failover-promoted", "Region its writes go to", { autocomplete: "off", spellcheck: "false", required: "" }, "A region holding replica buckets of this one's.");
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "confirm", novalidate: "", "aria-labelledby": "failover-title" },
    el("p", { id: "failover-title" }, el("strong", {}, "Fail a region over")),
    failed.row,
    promoted.row,
    el("div", { class: "actions" }, el("button", { type: "submit", class: "danger" }, "Fail over…")),
    said,
  );
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const [from, to] = [failed.input.value.trim(), promoted.input.value.trim()];
    for (const [input, value] of [[failed.input, from], [promoted.input, to]] as const) {
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
          const answer = await call("POST", `${BASE}/failover`, ignored, { region: from, promoted: to, reason });
          return done(screen, answer.ok ? null : answer, `Failed ${from} over to ${to}.`);
        },
        () => fill(area),
      ),
    );
  });
  return form;
}

/** The region failover card. */
export async function regionsCard(screen: Screen): Promise<HTMLElement> {
  const answer = await call("GET", BASE, readRegions);
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
