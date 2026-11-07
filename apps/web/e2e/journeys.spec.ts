/**
 * The four launch journeys (M10 §19), each against the real stack.
 *
 * A  first run: create a graph, open it, reach the upload step
 * B  Graph Explorer on the seeded fraud demo (read only)
 * C  entity resolution on a graph of its own: scan, review, approve
 * D  without AI: upload, build the model by hand, ingest, explore
 */

import path from "node:path";

import { type Page, expect, test } from "@playwright/test";

import { EXAMPLES, findEntity, fraudDemo, fraudGraph, uniqueName } from "./support";

/** The status bar counter, e.g. "12 entities loaded". */
async function loaded(page: Page, what: "entities" | "relationships"): Promise<number> {
  const text = await page
    .getByText(new RegExp(`^\\d+ ${what} loaded$`))
    .first()
    .textContent();
  return Number.parseInt(text ?? "0", 10);
}

const details = (page: Page) => page.getByRole("complementary", { name: "Details" });

test("A: first run creates a graph and reaches the upload step", async ({ page }) => {
  await page.goto("/");
  // A fresh install shows the welcome screen, otherwise "Your Graphs".
  await page
    .getByRole("link", { name: /^(Create Graph|New Graph)$/ })
    .first()
    .click();
  await expect(page).toHaveURL(/\/graphs\/new$/);

  const name = uniqueName("first run");
  await page.getByRole("textbox", { name: "Graph name" }).fill(name);
  await page.getByRole("radio", { name: /Upload CSV/ }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  // The upload step drops `?kind=csv` from the address once it has read it.
  await expect(page).toHaveURL(/\/graphs\/[0-9a-f-]{36}\/upload(\?kind=csv)?$/);
  await expect(page.getByLabel("Dataset files")).toBeAttached();

  // The new (draft) graph is listed and opens on its upload step.
  await page.goto("/");
  const graphs = page.getByRole("region", { name: "Graphs" });
  await graphs.getByRole("link", { name, exact: true }).click();
  await expect(page).toHaveURL(/\/graphs\/[0-9a-f-]{36}\/upload$/);
  await expect(page.getByLabel("Dataset files")).toBeAttached();
});

test("B: the fraud demo opens in the Graph Explorer", async ({ page, request }) => {
  const demo = await fraudDemo(request);
  const alice = await findEntity(request, demo.id, "Customer", "customer_id", "C001");

  await page.goto("/demos");
  await page
    .getByRole("region", { name: "Fraud Investigation status" })
    .getByRole("link", { name: /Open in Explorer/ })
    .click();
  await expect(page).toHaveURL(`/graphs/${demo.id}/explore?entity=${alice.id}`);
  await expect(details(page).getByRole("heading", { name: "Alice Tan" })).toBeVisible();
  // The canvas really renders (software WebGL), not its fallback.
  await expect(page.locator("canvas").first()).toBeVisible();
  await expect(page.getByText(/WebGL is unavailable/)).toHaveCount(0);

  // Search for a customer and open it.
  const search = page.getByRole("form", { name: "Entity search" });
  await search.getByLabel("Entity type").selectOption("Customer");
  await search.getByLabel("Property").selectOption("customer_id");
  await search.getByLabel("Value").fill("C002");
  await search.getByRole("button", { name: /Search/ }).click();
  await expect(page.getByText("1 match")).toBeVisible();
  await page
    .getByRole("list", { name: "Search results" })
    .getByRole("button", { name: /Budi Santoso/ })
    .click();
  await expect(details(page).getByRole("heading", { name: "Budi Santoso" })).toBeVisible();
  await expect(details(page).getByText("C002", { exact: true })).toBeVisible();
  await expect(details(page).getByText("Bandung", { exact: true })).toBeVisible();

  // Open Budi's account through the relationship list, then expand it:
  // its transactions are not loaded yet.
  await details(page).getByRole("button", { name: "Inspect OWNS relationship" }).first().click();
  await details(page).getByRole("button", { name: /A1003/ }).click();
  await expect(details(page).getByRole("heading", { name: "A1003" })).toBeVisible();
  const before = await loaded(page, "entities");
  await details(page)
    .getByRole("button", { name: /Expand/ })
    .click();
  await expect.poll(() => loaded(page, "entities")).toBeGreaterThan(before);

  // Shortest path from Budi's account to Alice: through the shared device.
  await details(page)
    .getByRole("button", { name: /Find Path/ })
    .click();
  await page.getByLabel("Target entity ID").fill(alice.id);
  await page.getByRole("button", { name: "Use", exact: true }).click();
  await page.getByRole("button", { name: /Find shortest path/ }).click();
  // A1003 ← OWNS Budi Santoso → USES DEVICE-921 ← USES Alice Tan
  await expect(page.getByText(/^3\s*hops$/).first()).toBeVisible();
  await expect(page.getByText("DEVICE-921", { exact: true }).first()).toBeVisible();
});

test("C: entity resolution links two records without changing the graph", async ({
  page,
  request,
}) => {
  const graph = await fraudGraph(request, uniqueName("resolution"), [
    "customers.csv",
    "customers_kyc.csv",
  ]);
  const canonical = await findEntity(request, graph.id, "Customer", "customer_id", "K-1004");
  const duplicate = await findEntity(request, graph.id, "Customer", "customer_id", "K-1005");

  await page.goto(`/graphs/${graph.id}/resolution`);
  const rules = page.getByRole("complementary", { name: "Resolution rules" });
  await expect(rules.getByRole("combobox", { name: "Entity type" })).toHaveValue("Customer");
  await rules.getByRole("button", { name: "Run Scan" }).click();
  await expect(page.getByRole("region", { name: "Scan coverage" })).toBeVisible();

  const queue = page.getByRole("region", { name: "Candidates" });
  await queue.getByRole("button", { name: /M\. Ikhwan Nasution/ }).click();
  const review = page.getByRole("region", { name: "Review" });
  const compare = review.getByRole("region", { name: "Compare entities" });
  await expect(compare).toContainText("ikhwan@example.com");
  await expect(compare).toContainText("IKHWAN@example.com");
  await expect(review).toContainText("not a probability");

  await review.getByRole("button", { name: "Same Entity" }).click();
  const chooser = review.getByRole("region", { name: "Choose the canonical entity" });
  await chooser.getByRole("radio", { name: /Muhammad Ikhwan Nasution/ }).click();
  await chooser.getByRole("button", { name: "Confirm same entity" }).click();

  // The canonical mapping, through the API ...
  await expect
    .poll(async () => {
      const response = await request.get(
        `/api/v1/graphs/${graph.id}/resolution/entities/${duplicate.id}`,
      );
      return (await response.json()).canonical_entity_id;
    })
    .toBe(canonical.id);
  // ... and in the Explorer, where both records stay separate nodes.
  await page.goto(`/graphs/${graph.id}/explore?entity=${duplicate.id}`);
  const identity = page.getByRole("region", { name: "Entity resolution" });
  await expect(identity).toContainText("Resolved identity");
  await expect(identity).toContainText("Muhammad Ikhwan Nasution");
  await expect(identity).toContainText("2 entity records linked");
});

test("D: without AI, a graph is built by hand and explored", async ({ page, request }) => {
  const status = await (await request.get("/api/v1/graph-builder/status")).json();
  test.skip(status.status === "ready", "the AI Graph Builder is configured on this stack");

  await page.goto("/graphs/new");
  await page.getByRole("textbox", { name: "Graph name" }).fill(uniqueName("manual"));
  await page.getByRole("radio", { name: /Upload CSV/ }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page).toHaveURL(/\/upload(\?kind=csv)?$/);

  await page
    .getByLabel("Dataset files")
    .setInputFiles([
      path.join(EXAMPLES, "fraud", "customers.csv"),
      path.join(EXAMPLES, "fraud", "devices.csv"),
    ]);
  const files = page.getByRole("region", { name: "Files" });
  await expect(files.getByText("Parsed")).toHaveCount(2, { timeout: 60_000 });
  await page.getByRole("button", { name: /Analyze Data/ }).click();
  await expect(page).toHaveURL(/\/builder\?sources=/);

  await expect(
    page.getByText(
      status.status === "not_configured"
        ? /AI Graph Builder is not configured/
        : /AI service cannot be reached/,
    ),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Generate Graph Model" })).toBeDisabled();
  await page.getByRole("button", { name: "Build manually" }).click();
  const entities = page.getByRole("list", { name: "Entities" });
  await expect(entities.getByText("Customer", { exact: true })).toBeVisible();
  await expect(entities.getByText("Device", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Generate Graph" }).click();
  await expect(page.getByText("Graph generated")).toBeVisible({ timeout: 90_000 });
  await page.getByRole("link", { name: /Explore Graph/ }).click();
  await expect(details(page).getByRole("heading").first()).toBeVisible();
  await expect.poll(() => loaded(page, "entities")).toBeGreaterThan(0);
});
