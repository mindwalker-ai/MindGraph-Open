import type { Metadata } from "next";

import { CreateGraph } from "@/components/create/CreateGraph";

export const metadata: Metadata = { title: "Create a Graph · MindGraph" };

export default function NewGraphPage() {
  return <CreateGraph />;
}
