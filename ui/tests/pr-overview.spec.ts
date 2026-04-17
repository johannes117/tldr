import { test, expect } from "@playwright/test";
import { installMocks, PR } from "./fixtures";

test("renders PR overview", async ({ page }) => {
  installMocks(page);
  await page.goto("/pr/1");
  await expect(page.getByRole("heading", { name: PR.pr.title })).toBeVisible();
  await expect(page.getByText(`${PR.slug} #${PR.pr.number}`)).toBeVisible();
  await expect(page.getByRole("link", { name: "Files" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Review" })).toBeVisible();
});
