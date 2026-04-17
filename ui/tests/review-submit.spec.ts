import { test, expect } from "@playwright/test";
import { installMocks } from "./fixtures";

test("select verdict and submit review", async ({ page }) => {
  const s = installMocks(page);
  await page.goto("/pr/1/review");

  await expect(page.getByRole("heading", { name: "Submit review" })).toBeVisible();

  // Pick "approve" radio
  await page.locator('input[type=radio][value=approve]').check();

  // Fill summary
  const ta = page.locator("textarea").first();
  await ta.fill("Looks good to me");

  // Submit
  await page.getByRole("button", { name: /Submit to GitHub/ }).click();

  // Success message surfaces
  await expect(page.getByText("submitted")).toBeVisible({ timeout: 5000 });
  // Verdict persisted to mock draft
  await expect.poll(() => s.draft.verdict).toBe("approve");
});
