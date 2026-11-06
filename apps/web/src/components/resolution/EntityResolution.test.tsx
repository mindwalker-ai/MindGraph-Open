// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { IdentityCard } from "@/components/explorer/IdentityCard";
import { EntityResolution } from "@/components/resolution/EntityResolution";
import { type ApiClient, ApiError } from "@/lib/api/client";
import type {
  CandidateDetail,
  CandidatePage,
  Entity,
  EntityIdentity,
  GraphProject,
  Ontology,
  ResolutionCandidate,
  ResolutionScan,
} from "@/lib/api/types";

vi.mock("next/link", () => ({
  default: ({ href, children, ...props }: { href: string; children: ReactNode }) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));
vi.mock("next/image", () => ({
  default: ({ alt }: { alt: string }) => <span data-alt={alt} />,
}));
vi.mock("next/navigation", () => ({
  usePathname: () => "/graphs/g1/resolution",
  useRouter: () => ({ push: vi.fn() }),
}));

afterEach(cleanup);

const GRAPH: GraphProject = {
  id: "g1",
  workspace_id: "w",
  name: "Fraud Investigation",
  description: null,
  status: "active",
  created_at: "2026-11-06T00:00:00Z",
  updated_at: "2026-11-06T00:00:00Z",
};

const ONTOLOGY: Ontology = {
  graph_id: "g1",
  version: 2,
  entity_types: [
    {
      id: "t1",
      name: "Customer",
      display_name: "Customer",
      description: null,
      properties: [
        { key: "customer_id", type: "string", required: true },
        { key: "name", type: "string", required: true },
        { key: "email", type: "string", required: false },
        { key: "phone", type: "string", required: false },
        { key: "age", type: "integer", required: false },
      ],
    },
    {
      id: "t2",
      name: "Device",
      display_name: "Device",
      description: null,
      properties: [{ key: "device_id", type: "string", required: true }],
    },
  ],
  relationship_types: [],
};

const customer = (id: string, properties: Entity["properties"]): Entity => ({
  id,
  graph_id: "g1",
  entity_type: "Customer",
  properties,
  property_types: Object.fromEntries(Object.keys(properties).map((k) => [k, "string"])),
});

const IKHWAN = customer("e1", {
  customer_id: "K-1004",
  name: "Muhammad Ikhwan Nasution",
  email: "ikhwan@example.com",
  phone: "+6281212345678",
});
const IKHWAN_B = customer("e2", {
  customer_id: "K-1005",
  name: "M. Ikhwan Nasution",
  email: "IKHWAN@example.com",
  phone: "081212345678",
});
const DEWI = customer("e3", { customer_id: "C004", name: "Dewi Kusuma" });
const DEWI_B = customer("e4", { customer_id: "K-1006", name: "Dewi Kusuma" });
const ENTITIES = [IKHWAN, IKHWAN_B, DEWI, DEWI_B];

const STRONG: ResolutionCandidate = {
  id: "c1",
  scan_id: "s1",
  graph_id: "g1",
  entity_type: "Customer",
  entity_a_id: "e1",
  entity_b_id: "e2",
  score: 0.9985,
  signals: [
    {
      property: "customer_id",
      rule: "exact_identifier",
      outcome: "no_match",
      matched: false,
      weight: 0,
    },
    {
      property: "email",
      rule: "normalized_email",
      outcome: "normalized",
      matched: true,
      weight: 0.75,
      note: "case_insensitive_local_part",
    },
    {
      property: "phone",
      rule: "normalized_phone",
      outcome: "normalized",
      matched: true,
      weight: 0.85,
    },
    {
      property: "name",
      rule: "fuzzy_name",
      outcome: "fuzzy",
      matched: true,
      similarity: 0.95,
      weight: 0.57,
      note: "initials_matched",
    },
  ],
  matched_rules: ["normalized_email:email", "normalized_phone:phone", "fuzzy_name:name"],
  status: "pending",
  created_at: "2026-11-06T00:00:00Z",
  updated_at: "2026-11-06T00:00:00Z",
  decided_at: null,
};

const WEAK: ResolutionCandidate = {
  ...STRONG,
  id: "c2",
  entity_a_id: "e3",
  entity_b_id: "e4",
  score: 0.6,
  signals: [
    { property: "email", rule: "normalized_email", outcome: "missing", matched: false, weight: 0 },
    {
      property: "name",
      rule: "fuzzy_name",
      outcome: "exact",
      matched: true,
      similarity: 1,
      weight: 0.6,
    },
  ],
  matched_rules: ["fuzzy_name:name"],
};

const SCAN: ResolutionScan = {
  scan_id: "s1",
  graph_id: "g1",
  entity_type: "Customer",
  ontology_version: 2,
  rules: [],
  max_entities: 400,
  entities_examined: 400,
  truncated: true,
  pairs_compared: 4667,
  comparison_budget: 20000,
  comparison_budget_reached: false,
  oversized_blocks: 0,
  candidate_limit_reached: false,
  candidates_found: 2,
  skipped: { already_pending: 0, previously_rejected: 0, already_linked: 0 },
  exhaustive: false,
  warnings: [{ code: "missing_country", property: "phone", rule: "normalized_phone", count: 3 }],
  duration_ms: 42,
  created_at: "2026-11-06T00:00:00Z",
};

const unresolved = (id: string): EntityIdentity => ({
  entity_id: id,
  canonical_entity_id: id,
  member_entity_ids: [id],
  is_canonical: true,
  resolved: false,
  resolution_type: null,
  entity_type: null,
  members: [],
  physical_graph_merged: false,
});

const page = (
  data: ResolutionCandidate[],
  counts = { pending: 2, accepted: 0, rejected: 0 },
): CandidatePage => ({ data, next_cursor: null, limit: 20, counts });

function detailOf(candidate: ResolutionCandidate): CandidateDetail {
  const find = (id: string) => ENTITIES.find((e) => e.id === id) ?? null;
  return {
    candidate,
    entity_a: find(candidate.entity_a_id),
    entity_b: find(candidate.entity_b_id),
    identity_a: unresolved(candidate.entity_a_id),
    identity_b: unresolved(candidate.entity_b_id),
    decisions: [],
  };
}

function api(overrides: Partial<Record<keyof ApiClient, unknown>> = {}): ApiClient {
  return {
    getGraph: vi.fn().mockResolvedValue(GRAPH),
    getOntology: vi.fn().mockResolvedValue(ONTOLOGY),
    listResolutionScans: vi.fn().mockResolvedValue([]),
    runResolutionScan: vi.fn().mockResolvedValue(SCAN),
    listResolutionCandidates: vi.fn().mockResolvedValue(page([STRONG, WEAK])),
    getResolutionCandidate: vi.fn(async (_g: string, id: string) =>
      detailOf(id === "c2" ? WEAK : STRONG),
    ),
    decideResolutionCandidate: vi.fn(),
    getSubgraph: vi.fn(async (_g: string, ids: string[]) => ({
      entities: ENTITIES.filter((e) => ids.includes(e.id)),
      relationships: [],
    })),
    getEntityIdentity: vi.fn(async (_g: string, id: string) => unresolved(id)),
    ...overrides,
  } as unknown as ApiClient;
}

const queue = () => screen.getByRole("region", { name: "Candidates" });
const review = () => screen.getByRole("region", { name: "Review" });

describe("EntityResolution", () => {
  it("suggests rules from the ontology and runs a bounded scan with them", async () => {
    const client = api();
    render(<EntityResolution graphId="g1" api={client} />);
    const rules = await screen.findByRole("complementary", { name: "Resolution rules" });
    await waitFor(() =>
      expect(
        (within(rules).getByRole("combobox", { name: "Entity type" }) as HTMLSelectElement).value,
      ).toBe("Customer"),
    );
    expect(
      (
        within(rules).getByRole("checkbox", {
          name: /Normalized email rule on email/,
        }) as HTMLInputElement
      ).checked,
    ).toBe(true);
    expect(
      (within(rules).getByRole("checkbox", { name: /Fuzzy name rule on name/ }) as HTMLInputElement)
        .checked,
    ).toBe(true);
    // `_id` properties are offered but not assumed to be identifiers.
    expect(
      (
        within(rules).getByRole("checkbox", {
          name: /Exact identifier rule on customer_id/,
        }) as HTMLInputElement
      ).checked,
    ).toBe(false);

    fireEvent.click(within(rules).getByRole("button", { name: "Run Scan" }));
    await waitFor(() => expect(client.runResolutionScan).toHaveBeenCalled());
    expect(client.runResolutionScan).toHaveBeenCalledWith("g1", {
      entity_type: "Customer",
      max_entities: 400,
      rules: [
        { property: "email", kind: "normalized_email", case_insensitive_local_part: true },
        { property: "phone", kind: "normalized_phone", country: "ID" },
        { property: "name", kind: "fuzzy_name", threshold: 0.9 },
      ],
    });
    const coverage = await screen.findByRole("region", { name: "Scan coverage" });
    expect(within(coverage).getByText("Partial scan")).toBeTruthy();
    expect(coverage.textContent).toContain("only the first 400 Customer entities were examined");
    expect(coverage.textContent).toContain("3 national numbers need a default country");
  });

  it("lists candidates with scores as decimals and compares both original records", async () => {
    render(<EntityResolution graphId="g1" api={api()} />);
    await within(queue()).findByText(/Muhammad Ikhwan Nasution/);
    expect(queue().textContent).toContain("1.00");
    expect(queue().textContent).toContain("0.60");
    expect(document.body.textContent).not.toMatch(/\d+%/);
    expect(screen.getByRole("tab", { name: /Pending/ }).textContent).toContain("2");

    const compare = await within(review()).findByRole("region", { name: "Compare entities" });
    expect(compare.textContent).toContain("ikhwan@example.com");
    expect(compare.textContent).toContain("IKHWAN@example.com");
    expect(compare.textContent).toContain("Normalized match");
    expect(compare.textContent).toContain("Fuzzy similarity 0.95");
    expect(compare.textContent).toContain("relies on an initial");
    expect(compare.textContent).toContain("No match");
    expect(review().textContent).toContain("ranking heuristic, not a probability");
    expect(review().textContent).toContain("strong evidence");

    fireEvent.click(within(queue()).getByRole("button", { name: /Dewi Kusuma/ }));
    await within(review()).findByText("weak evidence");
    expect(
      within(review()).getByRole("region", { name: "Compare entities" }).textContent,
    ).toContain("Missing data");
  });

  it("asks which record stays canonical and explains the mapping is logical", async () => {
    const client = api({
      decideResolutionCandidate: vi.fn().mockResolvedValue({}),
    });
    render(<EntityResolution graphId="g1" api={client} />);
    fireEvent.click(await within(review()).findByRole("button", { name: "Same Entity" }));
    const chooser = within(review()).getByRole("region", { name: "Choose the canonical entity" });
    expect(chooser.textContent).toContain(
      "This creates a logical identity mapping. Original graph entities and relationships will remain unchanged.",
    );
    const confirm = within(chooser).getByRole("button", { name: "Confirm same entity" });
    expect((confirm as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(within(chooser).getByRole("radio", { name: /M\. Ikhwan Nasution/ }));
    fireEvent.click(confirm);
    await waitFor(() =>
      expect(client.decideResolutionCandidate).toHaveBeenCalledWith("g1", "c1", {
        decision: "same_entity",
        canonical_entity_id: "e2",
      }),
    );
  });

  it("records different entities and reports conflicts", async () => {
    const decide = vi
      .fn()
      .mockRejectedValueOnce(
        new ApiError(409, "DECISION_CONFLICT", "this candidate already has a different decision"),
      );
    render(<EntityResolution graphId="g1" api={api({ decideResolutionCandidate: decide })} />);
    fireEvent.click(await within(review()).findByRole("button", { name: "Different Entities" }));
    await waitFor(() =>
      expect(decide).toHaveBeenCalledWith("g1", "c1", { decision: "different_entities" }),
    );
    expect((await within(review()).findByRole("alert")).textContent).toContain(
      "this candidate already has a different decision",
    );
  });

  it("filters the queue by entity when opened from the Graph Explorer", async () => {
    const client = api();
    render(<EntityResolution graphId="g1" initialEntityId="e3" api={client} />);
    await waitFor(() =>
      expect(client.listResolutionCandidates).toHaveBeenCalledWith("g1", {
        status: "pending",
        entity_id: "e3",
        limit: 20,
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Show all candidates" }));
    await waitFor(() =>
      expect(client.listResolutionCandidates).toHaveBeenLastCalledWith("g1", {
        status: "pending",
        entity_id: undefined,
        limit: 20,
      }),
    );
  });

  it("shows the review totals in the status bar", async () => {
    render(
      <EntityResolution
        graphId="g1"
        api={api({
          listResolutionCandidates: vi
            .fn()
            .mockResolvedValue(page([STRONG], { pending: 12, accepted: 3, rejected: 2 })),
        })}
      />,
    );
    const footer = screen.getByRole("contentinfo", { name: "Review status" });
    await waitFor(() => expect(footer.textContent).toContain("Pending: 12"));
    expect(footer.textContent).toContain("Accepted: 3");
    expect(footer.textContent).toContain("Rejected: 2");
  });
});

describe("IdentityCard", () => {
  it("shows a resolved identity and pending duplicates without percentages", async () => {
    const client = api({
      getEntityIdentity: vi.fn().mockResolvedValue({
        entity_id: "e2",
        canonical_entity_id: "e1",
        member_entity_ids: ["e1", "e2"],
        is_canonical: false,
        resolved: true,
        resolution_type: "manual",
        entity_type: "Customer",
        members: [],
        physical_graph_merged: false,
      }),
      listResolutionCandidates: vi.fn().mockResolvedValue(
        page([{ ...WEAK, entity_a_id: "e2", entity_b_id: "e4", score: 0.94 }], {
          pending: 1,
          accepted: 1,
          rejected: 0,
        }),
      ),
    });
    const open = vi.fn();
    render(<IdentityCard api={client} graphId="g1" entityId="e2" onOpenEntity={open} />);
    const card = await screen.findByRole("region", { name: "Entity resolution" });
    expect(card.textContent).toContain("Resolved identity");
    expect(card.textContent).toContain("Canonical: Muhammad Ikhwan Nasution");
    expect(card.textContent).toContain("2 entity records linked");
    expect(card.textContent).toContain("Potential duplicate");
    expect(card.textContent).toContain("score 0.94");
    expect(card.textContent).not.toMatch(/%/);
    fireEvent.click(within(card).getByRole("button", { name: /Muhammad Ikhwan Nasution/ }));
    expect(open).toHaveBeenCalledWith("e1");
    expect(
      within(card).getByRole("link", { name: "View Identity Group" }).getAttribute("href"),
    ).toBe("/graphs/g1/resolution?entity=e2");
  });

  it("renders nothing for an entity with no identity work", async () => {
    const client = api({
      listResolutionCandidates: vi
        .fn()
        .mockResolvedValue(page([], { pending: 0, accepted: 0, rejected: 0 })),
    });
    render(<IdentityCard api={client} graphId="g1" entityId="e3" onOpenEntity={vi.fn()} />);
    await waitFor(() => expect(client.getEntityIdentity).toHaveBeenCalled());
    expect(screen.queryByRole("region", { name: "Entity resolution" })).toBeNull();
  });
});
