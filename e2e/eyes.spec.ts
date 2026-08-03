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
  await expect(page.locator(".hero")).toBeVisible();
  await expect(page.locator(".census")).toBeVisible();
  await expect(page.getByText("Direction of travel")).toBeVisible();
  // Every quality cell speaks: the sentence is the accessible label.
  const cells = page.locator(".quality-cell");
  expect(await cells.count()).toBeGreaterThan(0);
  for (const cell of await cells.all()) {
    expect(await cell.getAttribute("title")).toBeTruthy();
  }
});

test("every surface answers without a loading stub left behind", async ({ page }) => {
  for (const surface of [
    "next", "work", "roadmap", "features", "tests",
    "library", "evidence", "timeline", "compare", "map",
  ]) {
    await page.goto(`/#${surface}`);
    await settled(page);
    await expect(
      page.locator("#stage h1, #stage h2, #stage .hero, #stage .lede").first(),
      `${surface} renders a heading`
    ).toBeVisible();
  }
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
  await expect(page).toHaveURL(/#features/);
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
