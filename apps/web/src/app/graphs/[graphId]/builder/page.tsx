import type { Metadata } from "next";
import { Suspense } from "react";

import { GraphBuilder } from "@/components/builder/GraphBuilder";

export const metadata: Metadata = { title: "Graph Builder · MindGraph" };

async function Builder({ params }: { params: Promise<{ graphId: string }> }) {
  const { graphId } = await params;
  return <GraphBuilder graphId={graphId} />;
}

export default function BuilderPage(props: PageProps<"/graphs/[graphId]/builder">) {
  return (
    <Suspense fallback={<p className="px-6 py-8 text-sm text-muted">Loading Graph Builder…</p>}>
      <Builder params={props.params} />
    </Suspense>
  );
}
