import type { Metadata } from "next";
import { connection } from "next/server";
import { Suspense } from "react";

import { McpIntegration } from "@/components/integrations/McpIntegration";
import { loopbackCoreUrl } from "@/lib/mcp/config";

export const metadata: Metadata = { title: "MCP Integration · MindGraph" };

async function Integration({ params }: PageProps<"/graphs/[graphId]/integrations/mcp">) {
  const { graphId } = await params;
  // Read at request time, so one image serves every environment. Inside
  // Docker, Core's URL is an internal name the MCP binary cannot use; the
  // snippets then suggest the loopback port `docker compose up` publishes.
  await connection();
  const defaultCoreUrl = loopbackCoreUrl(process.env.MINDGRAPH_CORE_URL);
  return <McpIntegration key={graphId} graphId={graphId} defaultCoreUrl={defaultCoreUrl} />;
}

export default function McpIntegrationPage(props: PageProps<"/graphs/[graphId]/integrations/mcp">) {
  return (
    <Suspense fallback={<div className="min-h-screen bg-bg" aria-busy />}>
      <Integration {...props} />
    </Suspense>
  );
}
