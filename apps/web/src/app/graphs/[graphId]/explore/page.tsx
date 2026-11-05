import type { Metadata } from "next";
import { Suspense } from "react";

import { GraphExplorer } from "@/components/explorer/GraphExplorer";

export const metadata: Metadata = { title: "Graph Explorer · MindGraph" };

async function Explorer({ params, searchParams }: PageProps<"/graphs/[graphId]/explore">) {
  const { graphId } = await params;
  const { entity, ask } = await searchParams;
  return (
    <GraphExplorer
      key={graphId}
      graphId={graphId}
      initialEntityId={typeof entity === "string" && entity ? entity : undefined}
      initialPanel={ask === "1" ? "ask" : "details"}
    />
  );
}

export default function ExplorePage(props: PageProps<"/graphs/[graphId]/explore">) {
  return (
    <Suspense fallback={<div className="h-screen bg-bg" aria-busy />}>
      <Explorer {...props} />
    </Suspense>
  );
}
