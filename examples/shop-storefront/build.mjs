// Bundle the storefront with esbuild, resolving the `@dxjs/*` packages from the
// sibling dxjs checkout (dxjs is a private, unpublished package — like DORM it is
// consumed as a sibling repo, not from a registry). Override its location with
// DXJS_DIR and the esbuild binary with ESBUILD_BIN.
//
//   node build.mjs    →    dist/app.js  (single ESM bundle for the browser)

import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";
import { mkdirSync } from "node:fs";

const here = dirname(fileURLToPath(import.meta.url));
const dxjs = process.env.DXJS_DIR || resolve(here, "../../../dxjs");
const esbuild = process.env.ESBUILD_BIN || join(dxjs, "node_modules/.bin/esbuild");
const pkg = (p) => join(dxjs, "packages", p, "src/index.ts");

mkdirSync(join(here, "dist"), { recursive: true });
execFileSync(
  esbuild,
  [
    join(here, "src/main.ts"),
    "--bundle",
    "--format=esm",
    "--platform=browser",
    "--target=es2021",
    `--alias:@dxjs/core=${pkg("core")}`,
    `--alias:@dxjs/dom=${pkg("dom")}`,
    `--alias:@dxjs/component=${pkg("component")}`,
    `--alias:@dxjs/transport=${pkg("transport")}`,
    `--outfile=${join(here, "dist/app.js")}`,
  ],
  { stdio: "inherit" },
);
console.log("built dist/app.js");
