// Browser E2E for the Seiva admin console (§10). Drives a real (headless) Chrome
// against the `admin-e2e` server: the page loads, shows NOTHING without a token
// (fail-closed), then — once a valid bearer is entered — reveals the registered,
// authorized `accounts` resource and drives its create form to a 200 that does
// NOT leak the password. Exits non-zero on the first failed assertion.
//
// Uses the SYSTEM Chrome (channel: "chrome"); no browser is downloaded.

import { chromium } from "playwright";
import assert from "node:assert/strict";

const BASE = process.env.ADMIN_E2E_BASE || "http://127.0.0.1:8099";
const TOKEN = "clerk-token";

function log(step) {
  console.log(`  ✓ ${step}`);
}

const browser = await chromium.launch({ channel: "chrome", headless: true });
let failed = false;
try {
  const page = await browser.newPage();
  page.on("pageerror", (e) => console.error("  page error:", e.message));

  // 1. The console loads and is self-identifying.
  await page.goto(`${BASE}/admin`, { waitUntil: "networkidle" });
  await assert.match(await page.title(), /Seiva/);
  await page.locator("header h1", { hasText: "Seiva Admin" }).waitFor();
  log("console page loads");

  // 2. With no token the registry is empty (fail-closed): no resource card.
  await page.locator("#intro", { hasText: /No resources are registered/ }).waitFor();
  assert.equal(await page.locator(".res").count(), 0, "no resource cards without a token");
  log("no token → no resources (fail-closed)");

  // 3. Enter a valid bearer; the console re-loads the registry with it and
  //    reveals the authorized `accounts` resource.
  await page.fill("#token", TOKEN);
  await page.locator("#token").blur(); // fire the `change` handler → reload
  const card = page.locator(".res", { has: page.locator("h2", { hasText: "Accounts" }) });
  await card.waitFor({ timeout: 5000 });
  log("valid token → the registered `accounts` resource appears");

  // 4. Drive the create form (POST .../create_user): fill username + password,
  //    Send, and assert a 2xx whose body carries the username but NOT the password.
  const form = page.locator("form.op", { hasText: "create_user" });
  await form.waitFor();
  const username = `e2e_${Date.now()}`;
  await form.locator('[data-k="username"]').fill(username);
  await form.locator('[data-k="password"]').fill("s3cr3t-p@ss");
  await form.locator('button[type="submit"]').click();
  const out = form.locator(".out");
  await out.waitFor();
  await page.waitForFunction(
    (el) => el && /\b20[01]\b/.test(el.textContent || ""),
    await out.elementHandle(),
    { timeout: 5000 },
  );
  const outText = await out.textContent();
  assert.match(outText, /\b20[01]\b/, `create should return 2xx, got: ${outText}`);
  assert.ok(outText.includes(username), "the response echoes the created username");
  assert.ok(!outText.includes("s3cr3t-p@ss"), "the response must NOT leak the password");
  log("create form → 2xx, username echoed, password not leaked");

  console.log("admin E2E: PASS");
} catch (err) {
  failed = true;
  console.error("admin E2E: FAIL\n", err);
} finally {
  await browser.close();
}
process.exit(failed ? 1 : 0);
