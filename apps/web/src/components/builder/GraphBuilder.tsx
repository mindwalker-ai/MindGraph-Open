"use client";

import Link from "next/link";
import { useEffect, useMemo, useRef, useState } from "react";

import { ProfileTable } from "@/components/builder/ProfileTable";
import { ReviewPanel } from "@/components/builder/ReviewPanel";
import {
  Badge,
  Button,
  ConfidenceBadge,
  ErrorNote,
  Panel,
  ProgressBar,
} from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient, newIdempotencyKey } from "@/lib/api/client";
import type {
  DataSource,
  GraphBuilderResult,
  GraphBuilderStatus,
  GraphProject,
  IngestionSummary,
  Job,
  Ontology,
} from "@/lib/api/types";
import {
  type BuilderDraft,
  fromSuggestion,
  manualDraft,
  ontologyToDraft,
  ontologyUnchanged,
  rebase,
  toSubmission,
  validateDraft,
} from "@/lib/builder/draft";
import { JobCancelled, waitForJob } from "@/lib/builder/jobs";
import { cn } from "@/lib/utils";

const STEPS = ["Data", "Analyze", "AI Model", "Review", "Generate"] as const;

type Generation =
  | { stage: "idle" }
  | { stage: "saving" }
  | { stage: "running"; job: Job | null }
  | { stage: "completed"; job: Job }
  | { stage: "failed"; message: string; ontologySaved: boolean };

function message(error: unknown): string {
  if (error instanceof ApiError) return `${error.message} (${error.code})`;
  return error instanceof Error ? error.message : String(error);
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

export interface GraphBuilderProps {
  graphId: string;
  /** Injectable for tests; defaults to the same-origin API. */
  api?: ApiClient;
  pollMs?: number;
}

/** The Graph Builder: upload, analyse, ask the AI for a model, review and
 * edit it, then generate the graph with the deterministic M4 pipeline. */
export function GraphBuilder({ graphId, api: injected, pollMs = 750 }: GraphBuilderProps) {
  const api = useMemo(() => injected ?? createApiClient(), [injected]);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const wait = (jobId: string, onUpdate?: (job: Job) => void) =>
    waitForJob(api, jobId, { intervalMs: pollMs, isCancelled: () => !alive.current, onUpdate });

  const [graph, setGraph] = useState<GraphProject | null>(null);
  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [aiStatus, setAiStatus] = useState<GraphBuilderStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [file, setFile] = useState<File | null>(null);
  const [source, setSource] = useState<DataSource | null>(null);
  const [busy, setBusy] = useState<"upload" | "analyze" | "suggest" | null>(null);
  const [stepError, setStepError] = useState<string | null>(null);
  const [suggestion, setSuggestion] = useState<{ job: Job | null; result?: GraphBuilderResult }>({
    job: null,
  });

  const [draft, setDraft] = useState<BuilderDraft | null>(null);
  const [baseVersion, setBaseVersion] = useState<number | null>(null);
  const [generation, setGeneration] = useState<Generation>({ stage: "idle" });
  const ingestionKey = useRef<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [g, o] = await Promise.all([api.getGraph(graphId), api.getOntology(graphId)]);
        if (cancelled) return;
        setGraph(g);
        setOntology(o);
      } catch (error) {
        if (!cancelled) setLoadError(message(error));
      }
      try {
        const status = await api.graphBuilderStatus();
        if (!cancelled) setAiStatus(status);
      } catch {
        if (!cancelled) setAiStatus({ status: "unavailable", configured: true });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [api, graphId]);

  const profile = source?.profile ?? null;
  const fields = useMemo(() => profile?.fields.map((f) => f.name) ?? [], [profile]);
  const errors = useMemo(() => (draft ? validateDraft(draft, fields) : []), [draft, fields]);
  const step = !source
    ? 0
    : !profile
      ? 1
      : !draft
        ? 2
        : generation.stage === "idle" ||
            generation.stage === "saving" ||
            (generation.stage === "failed" && !generation.ontologySaved)
          ? 3
          : 4;

  async function upload() {
    if (!file) return;
    setBusy("upload");
    setStepError(null);
    try {
      setSource(await api.uploadDataSource(graphId, file));
    } catch (error) {
      setStepError(message(error));
    } finally {
      setBusy(null);
    }
  }

  async function analyze() {
    if (!source) return;
    setBusy("analyze");
    setStepError(null);
    try {
      const accepted = await api.analyze(graphId, source.id);
      const job = await wait(accepted.job_id);
      if (job.status !== "completed") {
        setStepError(job.error ? `${job.error.message} (${job.error.code})` : "Analysis failed.");
      }
      setSource(await api.getDataSource(graphId, source.id));
    } catch (error) {
      if (!(error instanceof JobCancelled)) setStepError(message(error));
    } finally {
      if (alive.current) setBusy(null);
    }
  }

  function startDraft(next: BuilderDraft) {
    setDraft(next);
    setBaseVersion(ontology?.version ?? null);
    setGeneration({ stage: "idle" });
    ingestionKey.current = null;
  }

  async function suggest() {
    if (!source || !ontology) return;
    setBusy("suggest");
    setStepError(null);
    setSuggestion({ job: null });
    try {
      const accepted = await api.requestSuggestion(graphId, source.id, newIdempotencyKey());
      const job = await wait(accepted.job_id, (update) => setSuggestion({ job: update }));
      if (job.status === "completed" && job.result && "suggestion" in job.result) {
        setSuggestion({ job, result: job.result });
        startDraft(fromSuggestion(job.result.suggestion, ontologyToDraft(ontology)));
      } else {
        setStepError(
          job.error
            ? `The AI suggestion failed: ${job.error.message} (${job.error.code})`
            : "The AI suggestion failed.",
        );
      }
    } catch (error) {
      if (!(error instanceof JobCancelled)) setStepError(message(error));
    } finally {
      if (alive.current) setBusy(null);
    }
  }

  async function generate() {
    if (!draft || !source || !ontology || errors.length > 0) return;
    const { ontology: next, mapping } = toSubmission(draft);
    const ontologySaved = generation.stage === "failed" && generation.ontologySaved;
    ingestionKey.current ??= newIdempotencyKey();
    if (!ontologySaved) {
      setGeneration({ stage: "saving" });
      try {
        const current = await api.getOntology(graphId);
        if (current.version !== baseVersion) {
          setGeneration({
            stage: "failed",
            ontologySaved: false,
            message: `The ontology changed (now v${current.version}) since this draft was created. Discard the draft and start again.`,
          });
          return;
        }
        if (!ontologyUnchanged(draft.base, next)) {
          // The server re-checks the version atomically.
          const created = await api.createOntology(graphId, next, baseVersion ?? undefined);
          setOntology(created);
          // Later retries and edits build on the saved version.
          setBaseVersion(created.version);
          setDraft(rebase(draft, ontologyToDraft(created)));
        }
      } catch (error) {
        setGeneration({
          stage: "failed",
          ontologySaved: false,
          message:
            error instanceof ApiError && error.status === 409
              ? "The ontology changed since this draft was created. Discard the draft and start again."
              : `The ontology could not be saved: ${message(error)}`,
        });
        return;
      }
    }
    let completed = false;
    try {
      setGeneration({ stage: "running", job: null });
      const accepted = await api.startIngestion(graphId, source.id, mapping, ingestionKey.current);
      const job = await wait(accepted.job_id, (update) =>
        setGeneration({ stage: "running", job: update }),
      );
      if (job.status === "completed") {
        setGeneration({ stage: "completed", job });
        ingestionKey.current = null;
        completed = true;
      } else {
        ingestionKey.current = null;
        setGeneration({
          stage: "failed",
          ontologySaved: true,
          message: job.error
            ? `Graph generation failed: ${job.error.message} (${job.error.code})`
            : "Graph generation failed.",
        });
      }
    } catch (error) {
      if (error instanceof JobCancelled) return;
      setGeneration({
        stage: "failed",
        ontologySaved: true,
        message: `Ontology saved successfully. Graph generation could not start: ${message(error)}`,
      });
    }
    if (completed) {
      // Only refreshes the status badge: a failure here must not turn a
      // finished ingestion into a failed one (and invite a second ingestion).
      try {
        const refreshed = await api.getGraph(graphId);
        if (alive.current) setGraph(refreshed);
      } catch {
        // Keep the previous graph details.
      }
    }
  }

  if (loadError) {
    return (
      <main className="mx-auto max-w-5xl px-6 py-10">
        <ErrorNote>{loadError}</ErrorNote>
      </main>
    );
  }

  const aiReady = aiStatus?.status === "ready";

  return (
    <main className="mx-auto flex max-w-7xl flex-col gap-5 px-6 py-8">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <Link href="/" className="font-mono text-xs uppercase tracking-wider text-muted">
            ← Graphs
          </Link>
          <h1 className="text-xl font-semibold">{graph?.name ?? "Graph"} · Graph Builder</h1>
        </div>
        <div className="flex items-center gap-2">
          {graph && (
            <Badge tone={graph.status === "active" ? "success" : "neutral"}>{graph.status}</Badge>
          )}
          {ontology && <Badge>Ontology v{ontology.version}</Badge>}
        </div>
      </header>

      <ol className="flex flex-wrap gap-2" aria-label="Steps">
        {STEPS.map((label, index) => (
          <li
            key={label}
            aria-current={index === step ? "step" : undefined}
            className={cn(
              "rounded border px-2.5 py-1 font-mono text-xs",
              index === step ? "border-accent text-foreground" : "border-border text-muted",
              index < step && "text-success",
            )}
          >
            {index + 1} {label}
          </li>
        ))}
      </ol>

      {stepError && <ErrorNote>{stepError}</ErrorNote>}

      <Panel title="1 · Data">
        {source ? (
          <p className="text-sm">
            <span className="font-mono">{source.filename}</span>{" "}
            <span className="text-muted">
              · {source.kind.toUpperCase()} · {formatBytes(source.size_bytes)}
            </span>{" "}
            <Badge
              tone={
                source.status === "ready"
                  ? "success"
                  : source.status === "failed"
                    ? "danger"
                    : "neutral"
              }
            >
              {source.status}
            </Badge>
          </p>
        ) : (
          <div className="flex flex-wrap items-center gap-3">
            <input
              type="file"
              accept=".csv,.json"
              aria-label="Dataset file"
              className="text-sm"
              onChange={(event) => setFile(event.target.files?.[0] ?? null)}
            />
            <Button variant="primary" disabled={!file || busy !== null} onClick={upload}>
              {busy === "upload" ? "Uploading…" : "Upload"}
            </Button>
            <span className="text-xs text-muted">
              CSV with a header row, or a JSON array of flat objects.
            </span>
          </div>
        )}
      </Panel>

      {source && (
        <Panel title="2 · Analyze">
          {profile ? (
            <ProfileTable profile={profile} />
          ) : (
            <Button variant="primary" disabled={busy !== null} onClick={analyze}>
              {busy === "analyze" ? "Analyzing…" : "Analyze dataset"}
            </Button>
          )}
        </Panel>
      )}

      {profile && !draft && (
        <Panel title="3 · AI Model">
          <div className="flex flex-col gap-3">
            {aiStatus && !aiReady && (
              <p className="text-sm text-muted">
                {aiStatus.status === "not_configured"
                  ? "AI Graph Builder is not configured."
                  : "The AI service is unavailable right now."}{" "}
                You can still build the ontology and mapping manually.
              </p>
            )}
            {aiReady && (
              <p className="text-sm text-muted">
                Sends the field names, inferred types and the sample values shown above (not the
                file) to <span className="font-mono">{aiStatus?.model}</span>. Nothing is saved
                until you generate the graph.
              </p>
            )}
            <div className="flex flex-wrap gap-2">
              <Button variant="primary" disabled={!aiReady || busy !== null} onClick={suggest}>
                {busy === "suggest" ? "Generating graph model…" : "Generate Graph Model"}
              </Button>
              <Button
                disabled={busy !== null || !ontology}
                onClick={() =>
                  ontology && startDraft(manualDraft(profile, ontologyToDraft(ontology)))
                }
              >
                Build manually
              </Button>
            </div>
            {busy === "suggest" && (
              <div className="flex flex-col gap-1" aria-live="polite">
                <ProgressBar value={suggestion.job?.progress ?? 0} label="AI suggestion progress" />
                <span className="text-xs text-muted">
                  AI suggestion {suggestion.job?.status ?? "queued"}…
                </span>
              </div>
            )}
          </div>
        </Panel>
      )}

      {draft && step === 3 && (
        <>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-sm text-muted">
              {draft.origin === "ai" ? (
                <>
                  AI suggestion <ConfidenceBadge value={draft.confidence} /> — review and edit
                  before generating. Confidence is the model&apos;s own estimate, not a guarantee.
                </>
              ) : (
                "Manual draft — map fields to entity types, then generate."
              )}
            </p>
            <Button variant="ghost" onClick={() => setDraft(null)}>
              Discard draft
            </Button>
          </div>
          <ReviewPanel draft={draft} fields={fields} onChange={setDraft} />
          <Panel title="5 · Generate">
            <div className="flex flex-col gap-3">
              {errors.length > 0 && (
                <ul
                  className="flex flex-col gap-1 text-sm text-warning"
                  aria-label="Draft problems"
                >
                  {errors.map((error) => (
                    <li key={error}>• {error}</li>
                  ))}
                </ul>
              )}
              {generation.stage === "failed" && <ErrorNote>{generation.message}</ErrorNote>}
              <div>
                <Button
                  variant="primary"
                  disabled={errors.length > 0 || generation.stage === "saving"}
                  onClick={generate}
                >
                  {generation.stage === "saving" ? "Saving ontology…" : "Generate Graph"}
                </Button>
              </div>
            </div>
          </Panel>
        </>
      )}

      {step === 4 && (
        <Panel title="5 · Generate">
          <GenerationView
            generation={generation}
            graphId={graphId}
            onRetry={generate}
            onBack={() => setGeneration({ stage: "idle" })}
          />
        </Panel>
      )}
    </main>
  );
}

function GenerationView({
  generation,
  graphId,
  onRetry,
  onBack,
}: {
  generation: Generation;
  graphId: string;
  onRetry: () => void;
  onBack: () => void;
}) {
  if (generation.stage === "running") {
    const job = generation.job;
    return (
      <div className="flex flex-col gap-2" aria-live="polite">
        <ProgressBar value={job?.progress ?? 0} label="Graph generation progress" />
        <span className="text-sm text-muted">
          {job ? `${job.status} · ${Math.round(job.progress * 100)}%` : "Starting ingestion…"}
        </span>
      </div>
    );
  }
  if (generation.stage === "completed") {
    const summary = generation.job.result as IngestionSummary;
    const counters: [string, number][] = [
      ["Records processed", summary.records_processed],
      ["Entities created", summary.entities_created],
      ["Entities matched", summary.entities_matched],
      ["Relationships created", summary.relationships_created],
    ];
    return (
      <div className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-success">Graph generated</h3>
        <dl className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          {counters.map(([label, value]) => (
            <div key={label} className="rounded border border-border px-3 py-2">
              <dt className="font-mono text-[11px] uppercase tracking-wider text-muted">{label}</dt>
              <dd className="font-mono text-lg">{value.toLocaleString()}</dd>
            </div>
          ))}
        </dl>
        <div>
          <Link
            href={`/graphs/${graphId}/explore`}
            className="inline-flex rounded bg-accent px-3 py-1.5 text-sm font-medium text-white"
          >
            Explore Graph
          </Link>
        </div>
      </div>
    );
  }
  if (generation.stage === "failed") {
    return (
      <div className="flex flex-col gap-3">
        <ErrorNote>{generation.message}</ErrorNote>
        <div className="flex gap-2">
          <Button variant="primary" onClick={onRetry}>
            Retry ingestion
          </Button>
          <Button variant="ghost" onClick={onBack}>
            Back to review
          </Button>
        </div>
      </div>
    );
  }
  return null;
}
