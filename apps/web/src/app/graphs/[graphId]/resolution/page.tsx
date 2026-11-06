import type { Metadata } from "next";
import { Suspense } from "react";

import { EntityResolution } from "@/components/resolution/EntityResolution";

export const metadata: Metadata = { title: "Entity Resolution · MindGraph" };

async function Resolution({ params, searchParams }: PageProps<"/graphs/[graphId]/resolution">) {
  const { graphId } = await params;
  const { entity, candidate } = await searchParams;
  return (
    <EntityResolution
      key={graphId}
      graphId={graphId}
      initialEntityId={typeof entity === "string" && entity ? entity : undefined}
      initialCandidateId={typeof candidate === "string" && candidate ? candidate : undefined}
    />
  );
}

export default function ResolutionPage(props: PageProps<"/graphs/[graphId]/resolution">) {
  return (
    <Suspense fallback={<div className="h-screen bg-bg" aria-busy />}>
      <Resolution {...props} />
    </Suspense>
  );
}
