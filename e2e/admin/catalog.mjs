// Browser E2E for the console's ROW CRUD over a real DORM-backed module
// (admin-data-e2e `Catalog` on PostgreSQL). Drives a headless system Chrome
// through the full loop — create → it appears in the list table → edit → delete →
// it is gone — all via the console UI calling the module's contract operations.
// Exits non-zero on the first failed assertion.

import { chromium } from "playwright";
import assert from "node:assert/strict";

const BASE = process.env.CATALOG_E2E_BASE || "http://127.0.0.1:8098";
const TOKEN = "clerk-token";
const log = (s) => console.log(`  ✓ ${s}`);

const browser = await chromium.launch({ channel: "chrome", headless: true });
let failed = false;
try {
  const page = await browser.newPage();
  page.on("dialog", (d) => d.accept()); // auto-accept the delete confirm()
  page.on("pageerror", (e) => console.error("  page error:", e.message));

  await page.goto(`${BASE}/admin`, { waitUntil: "networkidle" });
  await page.fill("#token", TOKEN);
  await page.locator("#token").blur();

  // The Catalogue resource renders as a table.
  const card = page.locator(".res", { has: page.locator("h2", { hasText: "Catalogue" }) });
  await card.waitFor({ timeout: 5000 });
  await card.locator("table.tbl").waitFor();
  log("Catalogue resource renders as an admin table");

  // CREATE via the form above the table.
  const name = `item_${Date.now()}`;
  const createForm = card.locator("form.op");
  await createForm.locator('[data-k="name"]').fill(name);
  await createForm.locator('[data-k="price"]').fill("1234");
  await createForm.locator('button[type="submit"]').click();
  const row = card.locator("table.tbl tbody tr", { hasText: name });
  await row.waitFor({ timeout: 5000 });
  log("create → the new row appears in the list");

  // EDIT: open the row editor, change the name, save.
  const edited = `${name}_edited`;
  await row.locator("button", { hasText: "Edit" }).click();
  const editInput = card.locator('table.tbl [data-k="name"]');
  await editInput.waitFor();
  await editInput.fill(edited);
  await card.locator("table.tbl button", { hasText: "Save" }).click();
  await card.locator("table.tbl tbody tr", { hasText: edited }).waitFor({ timeout: 5000 });
  log("edit → the row shows the updated value");

  // DELETE: remove the (edited) row; it disappears.
  const editedRow = card.locator("table.tbl tbody tr", { hasText: edited });
  await editedRow.locator("button", { hasText: "Delete" }).click();
  await editedRow.waitFor({ state: "detached", timeout: 5000 });
  log("delete → the row is gone from the list");

  console.log("catalog row-CRUD E2E: PASS");
} catch (err) {
  failed = true;
  console.error("catalog row-CRUD E2E: FAIL\n", err);
} finally {
  await browser.close();
}
process.exit(failed ? 1 : 0);
