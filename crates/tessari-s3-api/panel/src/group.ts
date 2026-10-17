//! One group of a space: who is in it and which buckets it is granted, with the
//! forms to change either, and deleting the group — which takes every grant it
//! carries away from every member, so it is confirmed by name with a reason.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, fill, mono } from "./dom.ts";
import { moment } from "./format.ts";
import { grants, members, type GroupPath } from "./group-parts.ts";
import { readGroupDetail } from "./groups-model.ts";
import { icon } from "./icons.ts";
import { ignored } from "./models.ts";
import { format } from "./route.ts";
import { settle } from "./row-region.ts";
import { failed, head, loading, type Screen } from "./screen.ts";

export async function group(screen: Screen, space: string, name: string): Promise<void> {
  const at: GroupPath = (rest) => `/spaces/${encodeURIComponent(space)}/groups/${encodeURIComponent(name)}${rest}`;
  loading(screen, name, "the group");
  const answer = await call("GET", at(""), readGroupDetail);
  if (!answer.ok) {
    failed(screen, name, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const found = answer.value;
  const region = el("div", {});
  const remove = el("button", { type: "button", class: "danger" }, icon("trash"), "Delete group");
  remove.addEventListener("click", () =>
    fill(
      region,
      el(
        "div",
        { class: "card" },
        confirmation(
        "delete-group",
        mono(`${space}/${found.name}`),
        `Every member loses what the group grants within five seconds. The ${found.members.length.toLocaleString()} memberships and ${found.grants.length.toLocaleString()} grants go with it.`,
        "Delete group",
        async (reason) => {
          const gone = await call("DELETE", at(""), ignored, { reason });
          if (!gone.ok) {
            return settle(screen, gone);
          }
          announce(`Deleted group ${found.name}.`);
          location.hash = format({ kind: "groups", space });
          return null;
        },
          () => {
            fill(region);
            remove.focus();
          },
        ),
      ),
    ),
  );
  const lede = el("span", {}, "In space ", el("a", { href: format({ kind: "groups", space }) }, mono(space)), ` · created ${moment(found.created)}`);
  fill(screen.main, head(found.name, lede, remove), region, members(screen, found, at), grants(screen, found, at));
}
