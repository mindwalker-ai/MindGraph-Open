import Image from "next/image";

import { cn } from "@/lib/utils";

const milestones = [
  { id: "M0", label: "Repository bootstrap", done: true },
  { id: "M1", label: "Domain foundation", done: true },
  { id: "M2", label: "GraphStore", done: false },
  { id: "M6", label: "Visual Graph Explorer", done: false },
];

export default function Home() {
  return (
    <main className="mx-auto flex min-h-screen max-w-3xl flex-col justify-center gap-10 px-6 py-16">
      <Image src="/logo.png" alt="MindGraph" width={212} height={39} priority />

      <section className="flex flex-col gap-4">
        <h1 className="text-4xl font-semibold tracking-tight sm:text-5xl">
          Turn messy data into an explorable knowledge graph.
        </h1>
        <p className="text-lg text-muted">
          Open-source Graph Intelligence for humans and AI agents.
        </p>
      </section>

      <section aria-labelledby="status-heading" className="rounded-lg border border-border p-5">
        <h2
          id="status-heading"
          className="mb-3 font-mono text-xs uppercase tracking-wider text-muted"
        >
          Development status
        </h2>
        <ul className="flex flex-col gap-2 text-sm">
          {milestones.map((milestone) => (
            <li key={milestone.id} className="flex items-center gap-3">
              <span
                aria-hidden
                className={cn(
                  "size-2 rounded-full",
                  milestone.done ? "bg-accent" : "border border-muted",
                )}
              />
              <span className="font-mono text-muted">{milestone.id}</span>
              <span className={cn(!milestone.done && "text-muted")}>{milestone.label}</span>
              <span className="sr-only">{milestone.done ? "(done)" : "(planned)"}</span>
            </li>
          ))}
        </ul>
      </section>
    </main>
  );
}
