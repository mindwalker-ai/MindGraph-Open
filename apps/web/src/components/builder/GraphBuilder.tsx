"use client";

/**
 * The AI Graph Builder (mockup 05). For the analysed files of `?sources=`:
 *
 * 1. Model: ask the AI for one model of all files together (or build it by
 *    hand);
 * 2. Review: entity types on the left, the ontology preview in the centre,
 *    the selected type's definition and mappings on the right;
 * 3. Generate: save the ontology (with `expected_version`), then ingest
 *    each file, creators first, through the unchanged M4 endpoints.
 *
 * Without files it is the ontology editor of the graph ("Start Empty").
 */

import {
  ArrowLeft,
  Check,
  CircleCheck,
  CircleX,
  FileCheck,
  FileSpreadsheet,
  LoaderCircle,
  Lock,
  Sparkles,
  Waypoints,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useMemo, useRef, useState } from "react";

import { EntityPanel } from "@/components/builder/EntityPanel";
import { Inspector } from "@/components/builder/Inspector";
import { OntologyCanvas, type Selection } from "@/components/builder/OntologyCanvas";
import { Rail } from "@/components/shell/AppShell";
import { Button, ErrorNote, Kicker, Meter } from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient, newIdempotencyKey } from "@/lib/api/client";
import type {
  GraphBuilderResult,
  GraphBuilderStatus,
  GraphProject,
  IngestionSummary,
  Job,
  Ontology,
} from "@/lib/api/types";
import {
  type BuilderDraft,
  type DatasetInfo,
  addRelationshipType,
  addType,
  fromSuggestion,
  manualDraft,
  ontologyToDraft,
  ontologyUnchanged,
  rebase,
  removeType,
  toSubmission,
  validateDraft,
} from "@/lib/builder/draft";
import { JobCancelled, waitForJob } from "@/lib/builder/jobs";
import { cn } from "@/lib/utils";

interface Run {
  dataSourceId: string;
  name: string;
  status: "pending" | "running" | "completed" | "failed";
  job?: Job;
  error?: string;
}

type Generation =
  | { stage: "idle" }
  | { stage: "saving" }
  | { stage: "running"; runs: Run[] }
  | { stage: "completed"; runs: Run[] }
  | { stage: "failed"; message: string; ontologySaved: boolean; runs: Run[] };

function message(error: unknown): string {
  if (error instanceof ApiError) return `${error.message} (${error.code})`;
  return error instanceof Error ? error.message : String(error);
}

/** The same labels MindGraph Core gives the AI: filenames, made unique. */
function labelled(sources: { id: string; filename: string; profile: DatasetInfo["profile"] }[]) {
  const seen = new Map<string, number>();
  return sources.map((source): DatasetInfo => {
    const count = (seen.get(source.filename) ?? 0) + 1;
    seen.set(source.filename, count);
    return {
      dataSourceId: source.id,
      name: count === 1 ? source.filename : `${source.filename} (${count})`,
      profile: source.profile,
    };
  });
}

function summary(runs: Run[]): IngestionSummary {
  return runs.reduce(
    (sum, run) => {
      const result = run.job?.result;
      if (!result || !("records_processed" in result)) return sum;
      return {
        records_processed: sum.records_processed + result.records_processed,
        entities_created: sum.entities_created + result.entities_created,
        entities_matched: sum.entities_matched + result.entities_matched,
        relationships_created: sum.relationships_created + result.relationships_created,
      };
    },
    { records_processed: 0, entities_created: 0, entities_matched: 0, relationships_created: 0 },
  );
}

export interface GraphBuilderProps {
  graphId: string;
  /** The analysed data sources to model (`?sources=`); empty for ontology-only editing. */
  sourceIds: string[];
  /** Injectable for tests; defaults to the same-origin API. */
  api?: ApiClient;
  pollMs?: number;
}

export function GraphBuilder({
  graphId,
  sourceIds,
  api: injected,
  pollMs = 750,
}: GraphBuilderProps) {
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
  const [datasets, setDatasets] = useState<DatasetInfo[] | null>(null);
  const [aiStatus, setAiStatus] = useState<GraphBuilderStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [suggesting, setSuggesting] = useState<{ progress: number } | null>(null);
  const [suggestError, setSuggestError] = useState<string | null>(null);
  const [result, setResult] = useState<GraphBuilderResult | undefined>();
  const [draft, setDraft] = useState<BuilderDraft | null>(null);
  const [baseVersion, setBaseVersion] = useState<number | null>(null);
  const [selection, setSelection] = useState<Selection>(null);
  const [generation, setGeneration] = useState<Generation>({ stage: "idle" });
  const ingestionKeys = useRef(new Map<string, string>());
  // Files already ingested for this model: never ingested twice.
  const completedRuns = useRef(new Map<string, Run>());

  const sourceKey = sourceIds.join(",");
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const ids = sourceKey ? sourceKey.split(",") : [];
        const [g, o, sources] = await Promise.all([
          api.getGraph(graphId),
          api.getOntology(graphId),
          Promise.all(ids.map((id) => api.getDataSource(graphId, id))),
        ]);
        if (cancelled) return;
        setGraph(g);
        setOntology(o);
        const unready = sources.filter((s) => s.status !== "ready" || !s.profile);
        if (unready.length > 0) {
          setLoadError(
            `${unready.map((s) => s.filename).join(", ")} ${unready.length === 1 ? "is" : "are"} not analysed. Go back to Upload Data.`,
          );
          return;
        }
        const infos = labelled(
          sources.map((s) => ({ id: s.id, filename: s.filename, profile: s.profile! })),
        );
        setDatasets(infos);
        if (infos.length === 0) {
          // Ontology-only editing: start from the graph's ontology.
          setDraft(manualDraft([], ontologyToDraft(o)));
          setBaseVersion(o.version);
        }
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
  }, [api, graphId, sourceKey]);

  const errors = useMemo(() => {
    if (!draft) return [];
    const found = validateDraft(draft);
    // Without files only the ontology is saved: mappings are not required.
    return draft.datasets.length === 0
      ? found.filter((e) => e !== "Map at least one entity.")
      : found;
  }, [draft]);

  function startDraft(next: BuilderDraft) {
    setDraft(next);
    setBaseVersion(ontology?.version ?? null);
    setSelection(null);
    setGeneration({ stage: "idle" });
    ingestionKeys.current.clear();
    completedRuns.current.clear();
  }

  async function suggest() {
    if (!datasets || !ontology) return;
    setSuggesting({ progress: 0 });
    setSuggestError(null);
    try {
      const accepted = await api.requestSuggestion(
        graphId,
        datasets.map((d) => d.dataSourceId),
        newIdempotencyKey(),
      );
      const job = await wait(accepted.job_id, (update) =>
        setSuggesting({ progress: update.progress }),
      );
      if (job.status === "completed" && job.result && "suggestion" in job.result) {
        setResult(job.result);
        startDraft(fromSuggestion(job.result.suggestion, ontologyToDraft(ontology), datasets));
      } else {
        setSuggestError(
          job.error
            ? `The AI suggestion failed: ${job.error.message} (${job.error.code})`
            : "The AI suggestion failed.",
        );
      }
    } catch (error) {
      if (!(error instanceof JobCancelled)) setSuggestError(message(error));
    } finally {
      if (alive.current) setSuggesting(null);
    }
  }

  async function generate() {
    if (!draft || !ontology || errors.length > 0) return;
    const { ontology: next, ingestions } = toSubmission(draft);
    const saved = generation.stage === "failed" && generation.ontologySaved;
    if (!saved) {
      setGeneration({ stage: "saving" });
      try {
        if (!ontologyUnchanged(draft.base, next)) {
          // The server checks the base version atomically (409 if it moved).
          const created = await api.createOntology(graphId, next, baseVersion ?? undefined);
          setOntology(created);
          setBaseVersion(created.version);
          setDraft(rebase(draft, ontologyToDraft(created)));
        }
      } catch (error) {
        setGeneration({
          stage: "failed",
          ontologySaved: false,
          runs: [],
          message:
            error instanceof ApiError && error.status === 409
              ? "The ontology changed since this model was created. Reload the page and review again."
              : `The ontology could not be saved: ${message(error)}`,
        });
        return;
      }
    }
    const runs: Run[] = ingestions.map(
      (plan) =>
        completedRuns.current.get(plan.dataSourceId) ?? {
          dataSourceId: plan.dataSourceId,
          name: plan.name,
          status: "pending",
        },
    );
    const show = () => alive.current && setGeneration({ stage: "running", runs: [...runs] });
    show();
    for (const [index, plan] of ingestions.entries()) {
      if (runs[index].status === "completed") continue;
      runs[index] = { ...runs[index], status: "running", error: undefined };
      show();
      const key = ingestionKeys.current.get(plan.dataSourceId) ?? newIdempotencyKey();
      ingestionKeys.current.set(plan.dataSourceId, key);
      try {
        const accepted = await api.startIngestion(graphId, plan.dataSourceId, plan.mapping, key);
        const job = await wait(accepted.job_id, (update) => {
          runs[index] = { ...runs[index], job: update };
          show();
        });
        // A finished job keeps its key: a new attempt needs a new one.
        ingestionKeys.current.delete(plan.dataSourceId);
        if (job.status !== "completed") {
          runs[index] = {
            ...runs[index],
            status: "failed",
            job,
            error: job.error ? `${job.error.message} (${job.error.code})` : "Ingestion failed.",
          };
          setGeneration({
            stage: "failed",
            ontologySaved: true,
            runs: [...runs],
            message: `Ontology saved. Ingesting ${plan.name} failed.`,
          });
          return;
        }
        runs[index] = { ...runs[index], status: "completed", job };
        completedRuns.current.set(plan.dataSourceId, runs[index]);
      } catch (error) {
        if (error instanceof JobCancelled) return;
        // Could not start: the same key is safe to retry.
        runs[index] = { ...runs[index], status: "failed", error: message(error) };
        setGeneration({
          stage: "failed",
          ontologySaved: true,
          runs: [...runs],
          message: `Ontology saved. Ingesting ${plan.name} could not start.`,
        });
        return;
      }
    }
    setGeneration({ stage: "completed", runs: [...runs] });
    // Only refreshes the status badge; a failure here changes nothing.
    try {
      const refreshed = await api.getGraph(graphId);
      if (alive.current) setGraph(refreshed);
    } catch {
      // Keep the previous graph details.
    }
  }

  const busy = generation.stage === "saving" || generation.stage === "running";
  const generating =
    generation.stage === "running" ||
    generation.stage === "completed" ||
    (generation.stage === "failed" && generation.ontologySaved);
  const totals = (datasets ?? []).reduce(
    (sum, d) => ({
      rows: sum.rows + d.profile.record_count,
      columns: sum.columns + d.profile.fields.length,
    }),
    { rows: 0, columns: 0 },
  );
  const uploadHref = `/graphs/${graphId}/upload${sourceKey ? `?sources=${sourceKey}` : ""}`;

  return (
    <div className="flex h-screen overflow-hidden">
      <Rail graphId={graphId} />
      <div className="flex min-w-0 flex-1 flex-col">
        <Topbar
          graphName={graph?.name}
          uploadHref={uploadHref}
          hasFiles={(datasets?.length ?? sourceIds.length) > 0}
          step={generating ? 4 : 3}
          done={generation.stage === "completed"}
        />

        <div className="flex min-h-0 flex-1">
          {loadError ? (
            <div className="m-8 flex max-w-xl flex-col gap-3">
              <ErrorNote>{loadError}</ErrorNote>
              <Link href={uploadHref} className="text-[13px] text-accent">
                ← Upload Data
              </Link>
            </div>
          ) : !datasets || !ontology ? (
            <p className="m-8 text-[13px] text-text-3">Loading…</p>
          ) : generating ? (
            <GenerationPanel
              graphId={graphId}
              generation={generation}
              onRetry={() => void generate()}
              onBack={() => setGeneration({ stage: "idle" })}
            />
          ) : !draft ? (
            <ModelStart
              datasets={datasets}
              aiStatus={aiStatus}
              suggesting={suggesting}
              error={suggestError}
              onSuggest={() => void suggest()}
              onManual={() => startDraft(manualDraft(datasets, ontologyToDraft(ontology)))}
            />
          ) : (
            // Locked while saving: an edit then would be lost when the save completes.
            <fieldset
              disabled={busy}
              aria-label="Draft review"
              className="m-0 flex min-w-0 flex-1 border-0 p-0"
            >
              <EntityPanel
                draft={draft}
                selectedKey={selection?.kind === "entity" ? selection.key : null}
                onSelect={(key) => setSelection({ kind: "entity", key })}
                onAdd={() => {
                  const next = addType(draft);
                  setDraft(next);
                  setSelection({ kind: "entity", key: next.types[next.types.length - 1].key });
                }}
                onRemove={(key) => {
                  setDraft(removeType(draft, key));
                  setSelection(null);
                }}
              />
              <OntologyCanvas
                draft={draft}
                selection={selection}
                onSelect={setSelection}
                locked={busy}
                onAddRelationship={(source, target) => {
                  const next = addRelationshipType(draft, source, target);
                  setDraft(next);
                  setSelection({
                    kind: "relationship",
                    key: next.relationshipTypes[next.relationshipTypes.length - 1].key,
                  });
                }}
              />
              <Inspector
                draft={draft}
                selection={selection}
                onSelect={setSelection}
                onChange={setDraft}
                errors={errors}
                result={result}
              />
            </fieldset>
          )}
        </div>

        <footer className="flex h-14 shrink-0 items-center gap-[10px] border-t border-border bg-panel px-4">
          <FileCheck size={14} className="text-text-2" />
          <span className="font-mono text-[11px] font-medium text-text-2">
            {datasets && datasets.length > 0
              ? `${datasets.length} file${datasets.length === 1 ? "" : "s"} · ${totals.rows.toLocaleString()} rows · ${totals.columns} columns analyzed`
              : "Ontology only · no files"}
          </span>
          <span className="flex-1" />
          {draft && !generating && errors.length > 0 ? (
            <button
              type="button"
              onClick={() => setSelection(null)}
              className="text-[12px] text-danger hover:underline"
            >
              {errors.length} problem{errors.length === 1 ? "" : "s"} to fix
            </button>
          ) : (
            <span className="hidden text-[12px] text-text-3 md:inline">
              Nothing is written until you click Generate Graph
            </span>
          )}
          {draft && !generating && (
            <Button
              variant="ghost"
              disabled={busy}
              onClick={() => {
                setDraft(null);
                setSelection(null);
              }}
            >
              Discard model
            </Button>
          )}
          <Link
            href={uploadHref}
            className="inline-flex h-8 items-center gap-2 rounded-[4px] border border-border-strong bg-raised px-3 text-[13px] font-semibold hover:bg-hover"
          >
            <ArrowLeft size={14} /> Back
          </Link>
          {!generating && (
            <Button
              variant="primary"
              size="lg"
              icon={<Waypoints size={14} />}
              disabled={!draft || errors.length > 0 || busy}
              title={errors.length ? errors[0] : undefined}
              onClick={() => void generate()}
            >
              {generation.stage === "saving"
                ? "Saving ontology…"
                : draft?.datasets.length === 0
                  ? "Save Ontology"
                  : "Generate Graph"}
            </Button>
          )}
        </footer>
        {generation.stage === "failed" && !generation.ontologySaved && (
          <div className="border-t border-border bg-panel px-4 py-2">
            <ErrorNote>{generation.message}</ErrorNote>
          </div>
        )}
      </div>
    </div>
  );
}

// --- top bar ----------------------------------------------------------------------------

function Topbar({
  graphName,
  uploadHref,
  hasFiles,
  step,
  done,
}: {
  graphName?: string;
  uploadHref: string;
  hasFiles: boolean;
  step: 3 | 4;
  done: boolean;
}) {
  const steps = ["Source", "Upload", "Model", "Generate"];
  return (
    <header className="flex h-12 shrink-0 items-center gap-[10px] border-b border-border bg-panel px-4">
      <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-[10px] text-[13px]">
        <Link href="/graphs/new" className="text-text-2 hover:text-text">
          Create Graph
        </Link>
        <span className="text-text-3">/</span>
        <Link href={uploadHref} className="text-text-2 hover:text-text">
          Upload Data
        </Link>
        <span className="text-text-3">/</span>
        <span className="truncate font-semibold">
          AI Graph Builder{graphName ? ` · ${graphName}` : ""}
        </span>
      </nav>
      <span className="flex-1" />
      <ol aria-label="Steps" className="hidden items-center gap-[14px] md:flex">
        {steps.map((label, index) => {
          const number = index + 1;
          const complete = number < step || (number === step && done) || (number === 2 && hasFiles);
          const current = number === step && !done;
          const skipped = number === 2 && !hasFiles;
          return (
            <li
              key={label}
              aria-current={current ? "step" : undefined}
              className="flex items-center gap-1.5"
            >
              <span
                className={cn(
                  "flex h-[18px] w-[18px] items-center justify-center rounded-full font-mono text-[10px] font-medium",
                  current && "bg-accent text-on-accent",
                  complete && !current && "bg-hover text-text-2",
                  !current &&
                    !complete &&
                    "text-text-3 shadow-[inset_0_0_0_1px_var(--border-strong)]",
                )}
              >
                {complete && !current ? <Check size={11} /> : number}
              </span>
              <span
                className={cn(
                  "text-[12.5px]",
                  current ? "text-text" : "text-text-3",
                  skipped && "line-through",
                )}
              >
                {label}
              </span>
            </li>
          );
        })}
      </ol>
    </header>
  );
}

// --- step 3: model --------------------------------------------------------------------

function ModelStart({
  datasets,
  aiStatus,
  suggesting,
  error,
  onSuggest,
  onManual,
}: {
  datasets: DatasetInfo[];
  aiStatus: GraphBuilderStatus | null;
  suggesting: { progress: number } | null;
  error: string | null;
  onSuggest: () => void;
  onManual: () => void;
}) {
  const ready = aiStatus?.status === "ready";
  return (
    <div className="flex flex-1 items-start justify-center overflow-y-auto bg-[linear-gradient(var(--panel)_1px,transparent_1px),linear-gradient(90deg,var(--panel)_1px,transparent_1px)] bg-[size:32px_32px] px-6 py-12">
      <section className="flex w-full max-w-[560px] flex-col gap-5 rounded-[8px] border border-border bg-panel p-6">
        <div className="flex flex-col gap-1.5">
          <span className="font-mono text-[10px] font-medium tracking-[0.8px] text-accent">
            STEP 3 OF 4 · MODEL
          </span>
          <h1 className="text-[22px] font-bold">Build the graph model</h1>
          <p className="text-[13px] text-text-2">
            MindGraph proposes entity types, relationships and a mapping for every file, linking
            files that share identifiers. You review and edit everything before anything is written.
          </p>
        </div>
        <ul aria-label="Files" className="flex flex-col rounded-[6px] border border-border">
          {datasets.map((dataset, index) => (
            <li
              key={dataset.dataSourceId}
              className={cn(
                "flex h-10 items-center gap-[10px] px-3",
                index && "border-t border-border",
              )}
            >
              <FileSpreadsheet size={15} className="text-text-2" />
              <span className="flex-1 truncate font-mono text-[12px] font-medium">
                {dataset.name}
              </span>
              <span className="font-mono text-[10.5px] font-medium text-text-3">
                {dataset.profile.record_count.toLocaleString()} rows ·{" "}
                {dataset.profile.fields.length} columns
              </span>
            </li>
          ))}
        </ul>
        <div className="flex items-center gap-2 text-[12.5px]">
          <span
            className={cn(
              "h-1.5 w-1.5 rounded-full",
              ready ? "bg-ok" : aiStatus ? "bg-warn" : "bg-text-3",
            )}
          />
          {!aiStatus ? (
            <span className="text-text-3">Checking the AI Graph Builder…</span>
          ) : ready ? (
            <span className="text-text-2">
              AI ready · <span className="font-mono text-[11px]">{aiStatus.model}</span>
            </span>
          ) : aiStatus.status === "not_configured" ? (
            <span className="text-text-2">
              AI Graph Builder is not configured. Build the model manually, or set{" "}
              <span className="font-mono text-[11px]">MINDGRAPH_LLM_PROVIDER</span>.
            </span>
          ) : (
            <span className="text-text-2">The AI service cannot be reached right now.</span>
          )}
        </div>
        <p className="flex gap-2 rounded-[6px] border border-border bg-bg p-3 text-[12px] leading-[18px] text-text-3">
          <Lock size={13} className="mt-0.5 shrink-0" />
          The model receives field names, inferred types, up to 5 sample values per field and up to
          10 sample rows per file, plus the current ontology. Never the files themselves. Sample
          values can contain sensitive data: use a local model for such files.
        </p>
        {suggesting && (
          <div className="flex flex-col gap-2">
            <Meter value={Math.max(suggesting.progress, 0.05)} label="AI suggestion progress" />
            <span className="flex items-center gap-2 text-[12px] text-text-2">
              <LoaderCircle size={13} className="animate-spin text-accent" /> Asking the model… this
              can take a minute.
            </span>
          </div>
        )}
        {error && <ErrorNote>{error}</ErrorNote>}
        <div className="flex gap-2">
          <Button
            variant="primary"
            size="lg"
            icon={<Sparkles size={14} />}
            disabled={!ready || Boolean(suggesting) || datasets.length === 0}
            onClick={onSuggest}
          >
            Generate Graph Model
          </Button>
          <Button size="lg" disabled={Boolean(suggesting)} onClick={onManual}>
            Build manually
          </Button>
        </div>
      </section>
    </div>
  );
}

// --- step 4: generate -----------------------------------------------------------------

function GenerationPanel({
  graphId,
  generation,
  onRetry,
  onBack,
}: {
  graphId: string;
  generation: Generation;
  onRetry: () => void;
  onBack: () => void;
}) {
  const runs = "runs" in generation ? generation.runs : [];
  const totals = summary(runs);
  return (
    <div className="flex flex-1 items-start justify-center overflow-y-auto px-6 py-12">
      <section className="flex w-full max-w-[600px] flex-col gap-5 rounded-[8px] border border-border bg-panel p-6">
        <div className="flex flex-col gap-1.5">
          <Kicker className="text-accent">STEP 4 OF 4 · GENERATE</Kicker>
          <h1 className="text-[22px] font-bold">
            {generation.stage === "completed"
              ? "Graph generated"
              : generation.stage === "failed"
                ? "Generation stopped"
                : "Generating the graph…"}
          </h1>
          <p className="text-[13px] text-text-2">
            The ontology is saved; files are ingested one by one, files that create entity types
            first.
          </p>
        </div>
        <ul aria-label="Ingestions" className="flex flex-col rounded-[6px] border border-border">
          {runs.map((run, index) => (
            <li
              key={run.dataSourceId}
              className={cn("flex flex-col gap-1.5 px-3 py-2.5", index && "border-t border-border")}
            >
              <div className="flex items-center gap-[10px]">
                {run.status === "completed" ? (
                  <CircleCheck size={14} className="text-ok" />
                ) : run.status === "failed" ? (
                  <CircleX size={14} className="text-danger" />
                ) : run.status === "running" ? (
                  <LoaderCircle size={14} className="animate-spin text-accent" />
                ) : (
                  <span className="h-3.5 w-3.5 rounded-full shadow-[inset_0_0_0_1px_var(--border-strong)]" />
                )}
                <span className="flex-1 truncate font-mono text-[12px] font-medium">
                  {run.name}
                </span>
                {run.job?.result && "records_processed" in run.job.result && (
                  <span className="font-mono text-[10.5px] text-text-3">
                    {run.job.result.entities_created} created · {run.job.result.entities_matched}{" "}
                    matched · {run.job.result.relationships_created} relationships
                  </span>
                )}
              </div>
              {run.status === "running" && run.job && (
                <Meter value={run.job.progress} label={`Ingestion progress of ${run.name}`} />
              )}
              {run.error && <p className="text-[12px] text-danger">{run.error}</p>}
            </li>
          ))}
        </ul>
        {generation.stage === "completed" && (
          <dl className="grid grid-cols-4 gap-3">
            {[
              ["Records", totals.records_processed],
              ["Entities created", totals.entities_created],
              ["Entities matched", totals.entities_matched],
              ["Relationships", totals.relationships_created],
            ].map(([label, value]) => (
              <div
                key={label}
                className="flex flex-col rounded-[6px] border border-border bg-bg p-3"
              >
                <dt className="font-mono text-[10px] text-text-3">{label}</dt>
                <dd className="text-[20px] font-semibold">{Number(value).toLocaleString()}</dd>
              </div>
            ))}
          </dl>
        )}
        {generation.stage === "failed" && <ErrorNote>{generation.message}</ErrorNote>}
        <div className="flex gap-2">
          {generation.stage === "completed" && (
            <>
              <Link
                href={`/graphs/${graphId}/explore`}
                className="inline-flex h-9 items-center gap-2 rounded-[4px] bg-accent px-[18px] text-[14px] font-semibold text-on-accent hover:bg-accent/90"
              >
                <Waypoints size={14} /> Explore Graph
              </Link>
              <Link
                href="/"
                className="inline-flex h-9 items-center rounded-[4px] border border-border-strong bg-raised px-[18px] text-[14px] font-semibold hover:bg-hover"
              >
                Your graphs
              </Link>
            </>
          )}
          {generation.stage === "failed" && (
            <>
              <Button variant="primary" size="lg" onClick={onRetry}>
                Retry ingestion
              </Button>
              <Button size="lg" onClick={onBack}>
                Back to review
              </Button>
            </>
          )}
        </div>
      </section>
    </div>
  );
}
