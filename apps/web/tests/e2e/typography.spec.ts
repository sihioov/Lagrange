import { expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38182";

test("uses Pretendard for interface copy and Geist Mono for market data", async ({
  page,
  request,
}) => {
  await request.post(`${syntheticApiOrigin}/__test/scenario`, { data: { role: "owner" } });
  await page.setViewportSize({ height: 900, width: 1440 });
  await page.goto("/stock-beta");

  const bodyFont = await page
    .locator("body")
    .evaluate((element) => getComputedStyle(element).fontFamily);
  const headingFont = await page
    .getByRole("heading", { level: 1 })
    .evaluate((element) => getComputedStyle(element).fontFamily);
  const dataFont = await page
    .getByTestId("stock-beta-row-000001.KRX")
    .locator("td")
    .nth(1)
    .evaluate((element) => getComputedStyle(element).fontFamily);
  const pretendardLoaded = await page.evaluate(() =>
    document.fonts.check('16px "Pretendard Variable"'),
  );

  expect(bodyFont).toContain("Pretendard Variable");
  expect(headingFont).toContain("Pretendard Variable");
  expect(dataFont).toContain("GeistMono");
  expect(pretendardLoaded).toBe(true);
});
