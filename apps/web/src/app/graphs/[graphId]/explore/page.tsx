import type { Metadata } from "next";
import Link from "next/link";
import { Suspense } from "react";

export const metadata: Metadata = { title: "Graph Explorer · MindGraph" };

async function Placeholder({ params }: { params: Promise<{ graphId: string }> }) {
  const { graphId } = await params;
  return (
    <main className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-12">
      <h1 className="text-xl font-semibold">Graph Explorer</h1>
      <p className="text-sm text-muted">
        The visual Graph Explorer arrives in milestone M6. Your graph is stored and ready.
      </p>
      <div className="flex gap-3 text-sm">
        <Link href={`/graphs/${graphId}/builder`} className="text-accent">
          ← Back to the Graph Builder
        </Link>
        <Link href="/" className="text-muted">
          All graphs
        </Link>
      </div>
    </main>
  );
}

export default function ExplorePage(props: PageProps<"/graphs/[graphId]/explore">) {
  return (
    <Suspense fallback={null}>
      <Placeholder params={props.params} />
    </Suspense>
  );
}
