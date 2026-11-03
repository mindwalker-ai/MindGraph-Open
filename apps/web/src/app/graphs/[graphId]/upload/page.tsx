import type { Metadata } from "next";
import { Suspense } from "react";

import { UploadData } from "@/components/upload/UploadData";

export const metadata: Metadata = { title: "Upload Data · MindGraph" };

/** `?sources=a,b` restores files uploaded earlier (the page keeps it in step). */
function sourceList(raw: string | string[] | undefined): string[] {
  const value = Array.isArray(raw) ? raw.join(",") : (raw ?? "");
  return value.split(",").filter(Boolean);
}

async function Upload({ params, searchParams }: PageProps<"/graphs/[graphId]/upload">) {
  const { graphId } = await params;
  const { sources } = await searchParams;
  return <UploadData key={graphId} graphId={graphId} initialSources={sourceList(sources)} />;
}

export default function UploadPage(props: PageProps<"/graphs/[graphId]/upload">) {
  return (
    <Suspense fallback={<div className="min-h-screen bg-bg" aria-busy />}>
      <Upload {...props} />
    </Suspense>
  );
}
