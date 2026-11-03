import type { Metadata } from "next";
import { Suspense } from "react";

import { GraphBuilder } from "@/components/builder/GraphBuilder";

export const metadata: Metadata = { title: "AI Graph Builder · MindGraph" };

function sourceList(raw: string | string[] | undefined): string[] {
  const value = Array.isArray(raw) ? raw.join(",") : (raw ?? "");
  return value.split(",").filter(Boolean);
}

async function Builder({ params, searchParams }: PageProps<"/graphs/[graphId]/builder">) {
  const { graphId } = await params;
  const { sources } = await searchParams;
  const sourceIds = sourceList(sources);
  return (
    <GraphBuilder
      key={`${graphId}:${sourceIds.join(",")}`}
      graphId={graphId}
      sourceIds={sourceIds}
    />
  );
}

export default function BuilderPage(props: PageProps<"/graphs/[graphId]/builder">) {
  return (
    <Suspense fallback={<div className="h-screen bg-bg" aria-busy />}>
      <Builder {...props} />
    </Suspense>
  );
}
