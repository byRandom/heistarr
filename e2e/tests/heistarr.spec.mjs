// The main journey of a Synology user, in a real browser against the real
// server. Tests run in order and share one signed-in page.
import { test, expect } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const RUN = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", ".run");

test.describe.configure({ mode: "serial" });

/** @type {import('@playwright/test').Page} */
let page;
const errors = [];

test.beforeAll(async ({ browser }) => {
  page = await browser.newPage();
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    // HTTP error answers (401 before signing in, 400 for the short password
    // the tests try on purpose) are logged by the browser too; only real
    // script errors count.
    if (m.type() === "error" && !m.text().startsWith("Failed to load resource")) errors.push(m.text());
  });
});

test.afterAll(async () => {
  await page.close();
});

const nav = (name) => page.getByRole("navigation").getByRole("link", { name, exact: true });
const field = (label) => page.locator("label.field", { hasText: label }).locator("input");

test("first visit creates the owner", async () => {
  await page.goto("/");
  await expect(page.getByText("Create the owner account")).toBeVisible();
  await page.getByLabel("Username").fill("owner");
  await page.getByLabel("Password").fill("short");
  await page.getByRole("button", { name: "Create owner" }).click();
  // The browser's own minlength check or the server refuses it.
  await expect(page.getByRole("heading", { name: "Movies" })).toHaveCount(0);
  await page.getByLabel("Password").fill("correct horse battery");
  await page.getByRole("button", { name: "Create owner" }).click();
  await expect(page.getByRole("heading", { name: "Movies" })).toBeVisible();
  await expect(page.getByText("Your library is empty")).toBeVisible();
});

test("settings: TMDB, Download Station and an indexer, with Test buttons", async () => {
  await nav("Settings").click();
  await field("TMDB key").fill("wrong");
  const test = page.getByRole("button", { name: "Test", exact: true });
  await test.nth(0).click();
  await expect(page.getByText(/check the API key/)).toBeVisible();
  await field("TMDB key").fill("tmdb-test-key");
  await test.nth(0).click();
  await expect(page.getByText("TMDB key works")).toBeVisible();

  await field("Movies folder").fill(path.join(RUN, "library"));
  await page.getByLabel("Synology Download Station").check();
  await field("DSM address").fill("http://127.0.0.1:7103");
  await field("Username").fill("heistarr");
  await field(/^Password/).fill("nope");
  await field("Destination shared folder").fill("data/torrents");
  await field("Client path").fill("/data/torrents");
  await field("Same folder here").fill(path.join(RUN, "downloads"));
  await test.nth(1).click();
  await expect(page.getByText(/wrong username or password/)).toBeVisible();
  await field(/^Password/).fill("ds-test-password");
  await test.nth(1).click();
  await expect(page.getByText(/Connected to Download Station/)).toBeVisible();

  await page.getByLabel("Spanish (Spain)").check();
  await page.getByRole("button", { name: "Save settings" }).click();
  await expect(page.getByText("Saved.")).toBeVisible();

  await page.getByRole("button", { name: "Add indexer" }).click();
  await page.getByLabel("Name", { exact: true }).fill("Jackett");
  await page.getByLabel("Torznab URL").fill("http://127.0.0.1:7102/");
  await page.getByLabel("API key").fill("torznab-test-key");
  await page.getByRole("button", { name: "Test" }).last().click();
  await expect(page.getByText("Indexer answered")).toBeVisible();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText("Indexer saved.")).toBeVisible();

  // Secrets come back masked, and saving the mask keeps them.
  await page.reload();
  await expect(field("TMDB key")).toHaveValue("••••-key");
  await expect(field(/^Password/)).toHaveValue("••••word");
  await page.getByRole("button", { name: "Save settings" }).click();
  await expect(page.getByText("Saved.")).toBeVisible();
  await test.nth(0).click();
  await expect(page.getByText("TMDB key works")).toBeVisible();
});

test("add a movie and see why releases are accepted or rejected", async () => {
  await nav("Add movie").click();
  await page.getByLabel("Movie title").fill("dune");
  await page.getByRole("button", { name: "Search" }).click();
  await page.getByLabel("Search for a release as soon as it is added").uncheck();
  await page.getByRole("button", { name: "Add", exact: true }).first().click();
  await expect(page.getByRole("heading", { name: "Dune" })).toBeVisible();
  await expect(page.locator(".chip.wanted")).toBeVisible();

  await page.getByRole("button", { name: "Interactive search" }).click();
  const best = page.locator(".release.best");
  await expect(best).toContainText("Dune.2021.1080p.BluRay.x264.CASTELLANO-GOOD");
  // Spanish audio is required: the English WEB-DL is rejected with a reason.
  await expect(
    page.locator(".release", { hasText: "Dune.2021.1080p.WEB-DL.DDP5.1-NTb" }).locator(".why"),
  ).toContainText("audio English not wanted");
});

test("automatic search downloads with Download Station and imports", async () => {
  await page.getByRole("button", { name: "Search now" }).click();
  await expect(page.getByText(/Grabbed Dune\.2021\.1080p\.BluRay/)).toBeVisible();

  await nav("Downloads").click();
  await expect(page.getByText("Dune.2021.1080p.BluRay.x264.CASTELLANO-GOOD")).toBeVisible();

  await nav("Movies").click();
  await expect(page.locator(".chip.available")).toBeVisible({ timeout: 45_000 });
  const file = path.join(RUN, "library", "Dune (2021)", "Dune (2021).mkv");
  expect(fs.existsSync(file)).toBe(true);

  await nav("History").click();
  await expect(page.locator(".kind.imported")).toBeVisible();
  await expect(page.getByText(/Imported Dune \(2021\) \(hardlinked\)/)).toBeVisible();
});

test("the movie page shows the file, copyable", async () => {
  await nav("Movies").click();
  await page.locator(".poster-card", { hasText: "Dune" }).click();
  await expect(page.locator(".copy code", { hasText: "Dune (2021).mkv" })).toBeVisible();
  await expect(page.getByText("BluRay-1080p")).toBeVisible();
});

test("theme choice is applied and remembered", async () => {
  await nav("Settings").click();
  await page.getByLabel("Theme").selectOption("terminal");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "terminal");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "terminal");
  await page.getByLabel("Theme").selectOption("miami");
});

test("sign out, wrong password, sign in", async () => {
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByText("Sign in to continue.")).toBeVisible();
  await page.getByLabel("Username").fill("owner");
  await page.getByLabel("Password").fill("not the password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByText("wrong username or password")).toBeVisible();
  await page.getByLabel("Password").fill("correct horse battery");
  await page.getByRole("button", { name: "Sign in" }).click();
  // Back where we were (Settings), signed in.
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Sign out" })).toBeVisible();
});

test("removing a movie keeps the file unless asked", async () => {
  await nav("Movies").click();
  await page.locator(".poster-card", { hasText: "Dune" }).click();
  await page.getByRole("button", { name: "Remove" }).click();
  await page.locator(".panel").getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByText("Your library is empty")).toBeVisible();
  expect(fs.existsSync(path.join(RUN, "library", "Dune (2021)", "Dune (2021).mkv"))).toBe(true);
});

test("no errors in the browser console", async () => {
  expect(errors).toEqual([]);
});
