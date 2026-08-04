import { test, expect, Page } from "@playwright/test";

// The cockpit's client had no browser evidence: the Rust server is
// tested, but app.js was covered only by syntax, jargon, and CSS
// checks. This suite is the missing proof — it drives the real cockpit
// over the real store, and every test also asserts the console stayed
// clean, because a rendered page with a swallowed error is a false
// green.

let consoleErrors: string[] = [];

test.beforeEach(({ page }) => {
  consoleErrors = [];
  page.on("pageerror", (error) => consoleErrors.push(String(error)));
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
});

test.afterEach(() => {
  expect(consoleErrors, "the console stays clean").toEqual([]);
});

async function settled(page: Page) {
  await expect(page.locator(".loading")).toHaveCount(0);
}

test("the cockpit opens on Now and takes its reading", async ({ page }) => {
  await page.goto("/");
  // The verdict band: sentence, claim spine, sparkrow, trust stamp.
  await expect(page.locator(".verdict .hero")).toBeVisible();
  await expect(page.locator(".census")).toBeVisible();
  await expect(page.locator(".stamp")).toContainText("read only");
  // Every trend speaks: the sentence is the accessible label.
  const sparks = page.locator(".spark-item");
  expect(await sparks.count()).toBeGreaterThan(0);
  for (const item of await sparks.all()) {
    expect(await item.getAttribute("title")).toBeTruthy();
  }
  // And every trend opens the surface that holds its evidence — a trend
  // is never the end of the trail.
  await page.locator('.spark-item:has-text("Tests passing")').click();
  await expect(page).toHaveURL(/#proof\?tab=tests/);
});

test("every surface answers without a loading stub left behind", async ({ page }) => {
  for (const surface of [
    "work", "roadmap", "features", "proof", "time", "structure",
    // The retired addresses still land somewhere that renders.
    "next", "tests", "evidence", "library", "timeline", "compare", "map",
  ]) {
    await page.goto(`/#${surface}`);
    await settled(page);
    await expect(
      page.locator("#stage h1, #stage h2, #stage .hero, #stage .lede").first(),
      `${surface} renders a heading`
    ).toBeVisible();
  }
});

test("proof holds three registers under one roof", async ({ page }) => {
  await page.goto("/#proof");
  await settled(page);
  const tabs = page.locator(".proof-tabs button");
  await expect(tabs).toHaveCount(3);
  await tabs.filter({ hasText: "Evidence" }).click();
  await expect(page).toHaveURL(/#proof\?tab=evidence/);
  await settled(page);
  await expect(page.locator("#stage .claim").first()).toBeVisible();
  await tabs.filter({ hasText: "Artifacts" }).click();
  await settled(page);
  await expect(page.locator(".shelves button").first()).toBeVisible();
});

test("a suite opens its cases, and a case opens itself", async ({ page }) => {
  // Both of these crashed the client once: every case carries a `group`
  // naming its suite, and the truthy string made a case row render as
  // if it were a suite. Nothing expanded a suite until this test, so
  // the console stayed clean and the bug stayed hidden.
  await page.goto("/#proof?tab=tests");
  await settled(page);
  const suite = page.locator(".trow.holds").first();
  await suite.click();
  await expect(page.locator(".trow.child").first()).toBeVisible();
  await page.locator(".trow.child").first().click();
  const opened = page.locator(".trow-detail");
  await expect(opened).toBeVisible();
  // The case says what it is called in full and how it went.
  await expect(opened.locator(".case-full")).not.toBeEmpty();
});

test("search reaches through symbols to the declaring file", async ({ page }) => {
  await page.goto("/");
  await page.locator("#open-find").click();
  await page.keyboard.type("record_quality");
  await expect(page.getByText("declares record_quality")).toBeVisible();
});

test("the plain register retells the same facts and comes back", async ({ page }) => {
  await page.goto("/");
  await page.locator("#register").click();
  await expect(page).toHaveURL(/#roadmap/);
  await expect(page.locator(".plain-banner")).toBeVisible();
  // The nav recedes to the plain surfaces; the facts stay.
  const visible = page.locator(".rail button:visible");
  expect(await visible.count()).toBeLessThanOrEqual(3);
  await expect(page.locator("#stage .hero, #stage h1").first()).toBeVisible();
  await page.locator("#register").click();
  await expect(page.locator('.rail button[data-go="now"]')).toBeVisible();
});

test("compare travels by cause and restates the past loudly", async ({ page }) => {
  await page.goto("/#compare");
  await settled(page);
  const moments = page.locator(".moment");
  expect(await moments.count()).toBeGreaterThan(0);
  await moments.first().click();
  await settled(page);
  await expect(page.locator(".delta-strip")).toBeVisible();
  await expect(page.locator(".asof-banner")).toContainText("the past");
});

test("the approvals desk renders the decision when one waits", async ({ page }) => {
  await page.goto("/#work");
  await settled(page);
  // The desk only appears when a change is proposed; either way the
  // surface must say what is happening.
  await expect(page.locator("#stage h1").first()).toBeVisible();
  const desk = page.locator(".approval");
  if ((await desk.count()) > 0) {
    await expect(desk.first().locator(".fix, .command").first()).toBeVisible();
  }
});
