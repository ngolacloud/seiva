// Proof + screenshot: load the running storefront (served on its own origin) in
// system Chrome via Playwright, confirm it rendered a product fetched cross-origin
// from the shop API, and capture docs/getting-started/shop/img/storefront.png.
//
//   PLAYWRIGHT_PATH=<abs> STORE_BASE=http://127.0.0.1:5173 SHOT=<path> node shot.mjs

import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import assert from "node:assert/strict";

const PW =
  process.env.PLAYWRIGHT_PATH ||
  "/home/walter/.npm/_npx/e41f203b7505f1fb/node_modules/playwright/index.js";
const pw = await import(PW);
const chromium = pw.chromium || (pw.default && pw.default.chromium);

const BASE = process.env.STORE_BASE || "http://127.0.0.1:5173";
const SHOT = process.env.SHOT || "docs/getting-started/shop/img/storefront.png";
mkdirSync(dirname(SHOT), { recursive: true });

const browser = await chromium.launch({ channel: "chrome", headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 900, height: 760 }, deviceScaleFactor: 2 });
  const violations = [];
  page.on("console", (m) => m.type() === "error" && violations.push(m.text()));
  await page.goto(BASE, { waitUntil: "networkidle" });
  // The catalogue is fetched cross-origin from the shop API; wait for a product.
  await page.locator("ul.products li .name").first().waitFor({ timeout: 15000 });
  const names = await page.locator("ul.products li .name").allTextContents();
  assert.ok(names.length >= 1, "expected at least one product rendered from the API");
  console.log("products rendered:", JSON.stringify(names));
  await page.screenshot({ path: SHOT, fullPage: true });
  console.log("wrote", SHOT);

  // Navigate to a product detail (the dxjs router) and capture that view too.
  const detailShot = SHOT.replace(/\.png$/, "-detail.png");
  await page.locator("ul.products li .name").first().click();
  await page.locator("main button", { hasText: "Order one" }).waitFor({ timeout: 10000 });
  assert.match(page.url(), /\/product\//, "expected the router to navigate to /product/:id");
  console.log("navigated to", page.url());
  await page.screenshot({ path: detailShot, fullPage: true });
  console.log("wrote", detailShot);

  if (violations.length) console.log("console errors:", violations.slice(0, 3));
} finally {
  await browser.close();
}
