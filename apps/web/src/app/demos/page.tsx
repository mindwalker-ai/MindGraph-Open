import type { Metadata } from "next";

import { DemoScenarios } from "@/components/demos/DemoScenarios";

export const metadata: Metadata = { title: "Demo scenarios · MindGraph" };

export default function DemosPage() {
  return <DemoScenarios />;
}
