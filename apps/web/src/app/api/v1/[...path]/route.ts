/**
 * Same-origin proxy from the browser to MindGraph Core.
 *
 * Core is only reachable inside the Docker network (`MINDGRAPH_CORE_URL`,
 * default `http://localhost:8080` for local development). Bodies are
 * streamed both ways, so multipart uploads and server-sent events pass
 * through unchanged. Only the headers the API uses are forwarded.
 */

import type { NextRequest } from "next/server";

const FORWARDED_REQUEST_HEADERS = ["accept", "content-type", "idempotency-key", "x-request-id"];
const FORWARDED_RESPONSE_HEADERS = ["cache-control", "content-type", "location", "x-request-id"];

function coreUrl(): string {
  return process.env.MINDGRAPH_CORE_URL ?? "http://localhost:8080";
}

async function forward(request: NextRequest, context: RouteContext<"/api/v1/[...path]">) {
  const { path } = await context.params;
  const target = new URL(
    `/api/v1/${path.map(encodeURIComponent).join("/")}${request.nextUrl.search}`,
    coreUrl(),
  );
  const headers = new Headers();
  for (const name of FORWARDED_REQUEST_HEADERS) {
    const value = request.headers.get(name);
    if (value) headers.set(name, value);
  }
  const init: RequestInit & { duplex?: "half" } = {
    method: request.method,
    headers,
    redirect: "manual",
    cache: "no-store",
  };
  if (request.method !== "GET" && request.method !== "HEAD") {
    init.body = request.body;
    init.duplex = "half";
  }
  let upstream: Response;
  try {
    upstream = await fetch(target, init);
  } catch {
    return Response.json(
      { error: { code: "CORE_UNAVAILABLE", message: "MindGraph Core cannot be reached" } },
      { status: 502 },
    );
  }
  const responseHeaders = new Headers();
  for (const name of FORWARDED_RESPONSE_HEADERS) {
    const value = upstream.headers.get(name);
    if (value) responseHeaders.set(name, value);
  }
  return new Response(upstream.body, { status: upstream.status, headers: responseHeaders });
}

export { forward as GET, forward as POST };
