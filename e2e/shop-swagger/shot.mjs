// Capture Swagger-UI screenshots of the running shop-api, for the tutorial.
// Uses the SYSTEM Chrome via Playwright (no browser download). Run with the
// shop-api already serving at $BASE.
//
//   NODE_PATH=/home/walter/.npm/_npx/e41f203b7505f1fb/node_modules \
//   SHOT_DIR=docs/getting-started/shop/img node e2e/shop-swagger/shot.mjs

import { mkdirSync } from "node:fs";

const PW =
  process.env.PLAYWRIGHT_PATH ||
  "/home/walter/.npm/_npx/e41f203b7505f1fb/node_modules/playwright/index.js";
const pw = await import(PW);
const chromium = pw.chromium || (pw.default && pw.default.chromium);

const BASE = process.env.SHOP_SWAGGER_BASE || "http://127.0.0.1:8099";
const DIR = process.env.SHOT_DIR || "docs/getting-started/shop/img";
mkdirSync(DIR, { recursive: true });

const browser = await chromium.launch({ channel: "chrome", headless: true });
try {
  const page = await browser.newPage({
    viewport: { width: 1280, height: 1100 },
    deviceScaleFactor: 2,
  });
  await page.goto(`${BASE}/docs`, { waitUntil: "networkidle" });
  // Swagger UI renders the operations client-side after fetching /openapi.json.
  await page.locator(".swagger-ui .opblock").first().waitFor({ timeout: 15000 });

  await page.screenshot({ path: `${DIR}/swagger-overview.png`, fullPage: true });
  console.log("wrote swagger-overview.png");

  // Expand POST /v1/orders/place to show the auto-generated request schema.
  const place = page
    .locator(".opblock-summary", { hasText: "/v1/orders/place" })
    .first();
  if (await place.count()) {
    await place.click();
    await page.waitForTimeout(600);
    const block = page
      .locator(".opblock", { has: place })
      .first();
    await block.screenshot({ path: `${DIR}/swagger-place-order.png` });
    console.log("wrote swagger-place-order.png");
  } else {
    console.log("place-order operation not found; skipped detail shot");
  }
} finally {
  await browser.close();
}
