// Renders the console's source into the committed assets under ../assets/.
//
// Building or running the server never comes through here: the output is
// committed and embedded by include_str!, so `cargo build`, `docker build` and
// a release need no Node. The build is deterministic — run it twice and the
// second run changes no byte — which is what `verify.sh` checks.
//
// EVERY file under ../assets/ is written by this script; none is edited by hand.

import { build } from "esbuild";
import { copyFile, readFile } from "node:fs/promises";

const ASSETS = new URL("../assets/", import.meta.url);
const STATIC = new URL("./assets/", import.meta.url);

/** Nothing the page loads may come from off this node: a console must work on a network with no route out. */
function local(what, text) {
  if (/\b(src|href)\s*=\s*["']?(https?:)?\/\//i.test(text) || /url\(\s*["']?(https?:)?\/\//i.test(text)) {
    throw new Error(`${what} reaches for something off this node`);
  }
}

await build({
  entryPoints: [new URL("./src/console.ts", import.meta.url).pathname],
  outfile: new URL("console.js", ASSETS).pathname,
  bundle: true,
  format: "iife",
  platform: "browser",
  target: "es2022",
  charset: "utf8",
  legalComments: "inline",
  logLevel: "warning",
});

const script = await readFile(new URL("console.js", ASSETS), "utf8");
local("the emitted script", script);
// Does it PARSE: `Function` compiles the source without running it, so a
// script that would be a SyntaxError in the browser fails the build instead.
try {
  new Function(script);
} catch (failure) {
  throw new Error(`the emitted script does not parse: ${failure.message}`);
}
console.log(`console.js   ${script.length} bytes, parses`);

for (const name of ["index.html", "console.css"]) {
  const text = await readFile(new URL(name, STATIC), "utf8");
  local(name, text);
  await copyFile(new URL(name, STATIC), new URL(name, ASSETS));
  console.log(`${name.padEnd(12)} copied`);
}

// The product mark has one home, the repository's logo directory; the console
// serves that file rather than a copy that could drift from it.
const MARK = new URL("../../../assets/logo/tessaridb-s3-mark.svg", import.meta.url);
local("the product mark", await readFile(MARK, "utf8"));
await copyFile(MARK, new URL("favicon.svg", ASSETS));
console.log("favicon.svg  copied from assets/logo/tessaridb-s3-mark.svg");
