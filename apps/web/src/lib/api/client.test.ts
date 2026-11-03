import { describe, expect, it, vi } from "vitest";

import { ApiError, createApiClient } from "./client";

function reply(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

describe("createApiClient", () => {
  it("calls same-origin /api/v1 paths with the idempotency key", async () => {
    const fetcher = vi.fn().mockResolvedValue(reply(202, { job_id: "j1", status: "queued" }));
    const api = createApiClient("", fetcher);
    await expect(api.requestSuggestion("g1", ["d1", "d2"], "key-1")).resolves.toEqual({
      job_id: "j1",
      status: "queued",
    });
    const [url, init] = fetcher.mock.calls[0];
    expect(url).toBe("/api/v1/graphs/g1/graph-builder/suggestions");
    expect(init.method).toBe("POST");
    expect(init.headers["Idempotency-Key"]).toBe("key-1");
    expect(JSON.parse(init.body)).toEqual({ data_source_ids: ["d1", "d2"] });
  });

  it("sends ingestion mappings as JSON", async () => {
    const fetcher = vi.fn().mockResolvedValue(reply(202, { job_id: "j2", status: "queued" }));
    const mapping = { entities: [], relationships: [] };
    await createApiClient("", fetcher).startIngestion("g1", "d1", mapping, "key-2");
    const [, init] = fetcher.mock.calls[0];
    expect(JSON.parse(init.body)).toEqual({ mapping });
    expect(init.headers["Content-Type"]).toBe("application/json");
    expect(init.headers["Idempotency-Key"]).toBe("key-2");
  });

  it("raises ApiError with the envelope code", async () => {
    const fetcher = vi.fn().mockResolvedValue(
      reply(503, {
        error: { code: "AI_NOT_CONFIGURED", message: "not configured", request_id: "r1" },
      }),
    );
    const error = await createApiClient("", fetcher)
      .requestSuggestion("g", ["d"], "k")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ status: 503, code: "AI_NOT_CONFIGURED", requestId: "r1" });
  });

  it("turns non-JSON answers into ApiError", async () => {
    const html = new Response("<html>Bad Gateway</html>", { status: 502 });
    const fetcher = vi.fn().mockResolvedValue(html);
    const error = await createApiClient("", fetcher)
      .getGraph("g")
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({
      status: 502,
      code: "HTTP_ERROR",
      message: "Request failed with HTTP 502",
    });

    fetcher.mockResolvedValue(new Response("not json", { status: 200 }));
    await expect(createApiClient("", fetcher).getGraph("g")).rejects.toMatchObject({
      code: "INVALID_RESPONSE",
    });
  });

  it("sends the expected ontology version when given", async () => {
    const fetcher = vi.fn().mockImplementation(async () => reply(201, { version: 3 }));
    const draft = { entity_types: [], relationship_types: [] };
    const api = createApiClient("", fetcher);
    await api.createOntology("g1", draft, 2);
    await api.createOntology("g1", draft);
    expect(JSON.parse(fetcher.mock.calls[0][1].body)).toEqual({ ...draft, expected_version: 2 });
    expect(JSON.parse(fetcher.mock.calls[1][1].body)).toEqual(draft);
  });
});
