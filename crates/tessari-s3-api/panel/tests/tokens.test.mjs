// Whether every text colour the console draws clears WCAG 2.2 AA's 4.5:1 against
// every surface it is drawn on, in both themes. The pairs come from the sheet's own
// semantic map — each theme block's `--text`, `--ok`, … resolved through `var()` to
// the `oklch()` primitive — so the measured set cannot drift from what is drawn.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const css = readFileSync(new URL("../assets/console.css", import.meta.url), "utf8");

/** `--name: value;` declarations in one block body. */
function declarations(body) {
  const found = new Map();
  for (const match of body.matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)) {
    found.set(match[1], match[2].trim());
  }
  return found;
}

/** The body of the first rule whose selector is exactly `selector`, after `from`. */
function block(selector, from = 0) {
  const at = css.indexOf(`${selector} {`, from);
  assert.ok(at >= 0, `the sheet has no \`${selector}\` block`);
  const open = css.indexOf("{", at);
  return css.slice(open + 1, css.indexOf("}", open));
}

const primitives = declarations(block(":root"));
const light = declarations(block(":root", css.indexOf(":root {") + 1));
const dark = declarations(block(':root:not([data-theme="light"])'));

/** A token resolved through `var()` to its opaque `oklch(L C H)`. */
function colour(theme, name) {
  let value = theme.get(name) ?? primitives.get(name);
  for (let hops = 0; value?.startsWith("var(--") && hops < 4; hops += 1) {
    const next = value.slice(6, -1);
    value = theme.get(next) ?? primitives.get(next);
  }
  const parts = /^oklch\(([\d.]+) ([\d.]+) ([\d.]+)\)$/.exec(value ?? "");
  assert.ok(parts, `--${name} does not resolve to an opaque oklch() colour: ${value}`);
  return { l: Number(parts[1]), c: Number(parts[2]), h: Number(parts[3]) };
}

/** Relative luminance through OKLab to linear sRGB (Ottosson's matrices, WCAG's weights). */
function luminance({ l, c, h }) {
  const a = c * Math.cos((h * Math.PI) / 180);
  const b = c * Math.sin((h * Math.PI) / 180);
  const l1 = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m1 = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s1 = (l - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const clamp = (v) => Math.min(1, Math.max(0, v));
  const red = clamp(4.0767416621 * l1 - 3.3077115913 * m1 + 0.2309699292 * s1);
  const green = clamp(-1.2684380046 * l1 + 2.6097574011 * m1 - 0.3413193965 * s1);
  const blue = clamp(-0.0041960863 * l1 - 0.7034186147 * m1 + 1.707614701 * s1);
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function contrast(one, other) {
  const [lighter, darker] = [luminance(one), luminance(other)].sort((x, y) => y - x);
  return (lighter + 0.05) / (darker + 0.05);
}

test("the contrast instrument reproduces the published figures", () => {
  const white = { l: 1, c: 0, h: 0 };
  const black = { l: 0, c: 0, h: 0 };
  assert.ok(Math.abs(contrast(white, black) - 21) < 0.1, "white on black is 21:1 by definition");
  assert.ok(contrast(white, white) === 1, "a colour on itself is 1:1");
});

test("every text colour clears 4.5:1 on every surface it is drawn on, in both themes", () => {
  const inks = ["text", "text-muted", "accent-text", "ok", "warn", "bad"];
  const surfaces = ["surface", "surface-raised", "surface-sunken", "side"];
  const failing = [];
  for (const [name, theme] of [["light", light], ["dark", dark]]) {
    assert.ok(theme.has("text") && theme.has("surface"), `the ${name} block maps no semantic tokens`);
    const pairs = [
      ...inks.flatMap((ink) => surfaces.map((surface) => [ink, surface])),
      ["on-accent", "accent"],
      ["on-bad", "bad"],
    ];
    for (const [ink, surface] of pairs) {
      const ratio = contrast(colour(theme, ink), colour(theme, surface));
      if (ratio < 4.5) {
        failing.push(`${name}: --${ink} on --${surface} = ${ratio.toFixed(2)}:1`);
      }
    }
  }
  assert.deepEqual(failing, [], `below WCAG 2.2 AA 4.5:1:\n  ${failing.join("\n  ")}`);
});
