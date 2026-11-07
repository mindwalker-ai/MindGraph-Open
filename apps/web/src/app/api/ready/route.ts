/**
 * Whether MindGraph Core can serve requests, for the status dot in the app
 * shell. Proxies Core's `GET /ready` (PostgreSQL and HugeGraph reachable),
 * which the browser cannot reach directly inside Docker.
 */

import { connection } from "next/server";

export async function GET() {
  // Always answered at request time, never prerendered at build time.
  await connection();
  const core = process.env.MINDGRAPH_CORE_URL ?? "http://localhost:8080";
  try {
    const response = await fetch(new URL("/ready", core), {
      cache: "no-store",
      signal: AbortSignal.timeout(5_000),
    });
    return Response.json({ status: response.ok ? "ready" : "not_ready" });
  } catch {
    return Response.json({ status: "unreachable" });
  }
}
