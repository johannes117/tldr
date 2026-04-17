import { test, expect } from "@playwright/test";
import { installMocks } from "./fixtures";

test("compose and submit a line comment", async ({ page }) => {
  const s = installMocks(page);
  await page.goto("/pr/1/files");

  // Wait for diff to render
  await expect(page.locator("text=src/a.ts").first()).toBeVisible();

  // Hover a line row; click the "+" Add comment button (first available)
  const addBtn = page.locator('button[title="Add comment"]').first();
  await addBtn.waitFor({ state: "attached" });
  await addBtn.evaluate((b) => (b as HTMLButtonElement).click());

  // Composer textarea appears
  const ta = page.locator("textarea").first();
  await ta.waitFor();
  await ta.fill("nit: please rename this");

  // Click Add
  await page.getByRole("button", { name: "Add" }).click();

  // Comment should land in mocked state
  await expect.poll(() => s.draft.comments.length, { timeout: 5000 }).toBeGreaterThan(0);
  expect(s.draft.comments[0].body).toContain("nit: please rename");
});
