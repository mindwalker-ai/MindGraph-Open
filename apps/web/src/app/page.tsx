import Image from "next/image";

import { GraphHome } from "@/components/graphs/GraphHome";

export default function Home() {
  return (
    <main className="mx-auto flex max-w-4xl flex-col gap-8 px-6 py-12">
      <header className="flex flex-col gap-3">
        <Image src="/logo.png" alt="MindGraph" width={212} height={39} priority />
        <p className="text-muted">
          Turn messy data into an explorable knowledge graph. Upload a dataset, review the model,
          generate the graph.
        </p>
      </header>
      <GraphHome />
    </main>
  );
}
