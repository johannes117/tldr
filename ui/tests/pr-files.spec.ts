import { test, expect } from "@playwright/test";
import { installMocks } from "./fixtures";

test("renders diff with file tree and supports j/k/v", async ({ page }) => {
  const s = installMocks(page);
  await page.goto("/pr/1/files");

  await expect(page.getByText("Files (2)")).toBeVisible();
  await expect(page.locator("text=src/a.ts").first()).toBeVisible();
  await expect(page.locator("text=src/b.ts").first()).toBeVisible();

  // initial file shown is src/a.ts
  await expect(page.locator("header, div").filter({ hasText: /src\/a\.ts/ }).first()).toBeVisible();

  // press j -> next file
  await page.keyboard.press("j");
  // top bar path should update to src/b.ts
  await expect(page.locator(".font-mono").filter({ hasText: "src/b.ts" }).first()).toBeVisible();

  // press k -> previous
  await page.keyboard.press("k");
  await expect(page.locator(".font-mono").filter({ hasText: "src/a.ts" }).first()).toBeVisible();

  // press v -> mark viewed, sends PUT state
  await page.keyboard.press("v");
  await expect.poll(() => Object.keys(s.draft.file_state).length, { timeout: 5000 }).toBeGreaterThan(0);
});
