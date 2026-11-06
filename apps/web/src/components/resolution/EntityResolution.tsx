"use client";

/**
 * The Entity Resolution workspace (M9): configure matching rules, run a
 * bounded scan, review potential duplicates side by side, and record
 * decisions as logical identity mappings.
 *
 * Every number on this page comes from MindGraph Core. Scan coverage is
 * shown as reported, including truncation and skipped work, so a partial
 * scan is never presented as exhaustive. Approving never merges graph
 * entities: the graph, Graph Explorer, Ask MindGraph and MCP keep the
 * original records.
 */

import { CopyCheck, Info, LoaderCircle, Play, Plus, Trash2, X } from "lucide-react";
import Link from "next/link";
import { useEffect, useMemo, useState } from "react";

import { CandidateReview } from "@/components/resolution/CandidateReview";
import { AppShell } from "@/components/shell/AppShell";
import { Badge, Button, ErrorNote, Field, Kicker, inputClass } from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient } from "@/lib/api/client";
import {
  type CandidateDetail,
  type CandidatePage,
  type CandidateStatus,
  type DecisionRequest,
  type Entity,
  type GraphProject,
  type Ontology,
  RESOLUTION_LIMITS,
  type ResolutionCandidate,
  type ResolutionScan,
  type RuleKind,
  type ScanWarning,
} from "@/lib/api/types";
import { entityLabel, shortId } from "@/lib/graph/transform";
import {
  RULE_KINDS,
  type RuleDraft,
  evidenceStrength,
  formatScore,
  kindInfo,
  kindsFor,
  matchedSummary,
  ruleDraft,
  rulesToRequest,
  suggestRules,
} from "@/lib/resolution/rules";
import { cn } from "@/lib/utils";

function message(error: unknown): string {
  return error instanceof Error ? error.message : "Something went wrong";
}

const TABS: { status: CandidateStatus; label: string }[] = [
  { status: "pending", label: "Pending" },
  { status: "accepted", label: "Accepted" },
  { status: "rejected", label: "Rejected" },
];

/** A scan warning as a sentence, e.g. "3 values are not valid …". */
function warningText({ code, count }: ScanWarning): string {
  const one = count === 1;
  switch (code) {
    case "oversized_blocks":
      return `${count} ${one ? "block of a very common value was" : "blocks of very common values were"} skipped (too large to compare)`;
    case "invalid_values":
      return `${count} ${one ? "value is" : "values are"} not valid and ${one ? "was" : "were"} not compared`;
    case "missing_country":
      return `${count} national ${one ? "number needs" : "numbers need"} a default country`;
    case "integer_phone":
      return `${count} ${one ? "phone is" : "phones are"} stored as ${one ? "an integer" : "integers"} and ${one ? "was" : "were"} not compared (leading zeros may be lost)`;
    case "values_too_long":
      return `${count} ${one ? "value is" : "values are"} too long to compare`;
    case "type_mismatch":
      return `${count} ${one ? "value has" : "values have"} another type than the ontology says`;
  }
}

type OntologyType = Ontology["entity_types"][number];

// --- rules -----------------------------------------------------------------------------

function RuleRow({
  rule,
  properties,
  onChange,
  onRemove,
}: {
  rule: RuleDraft;
  properties: OntologyType["properties"];
  onChange(rule: RuleDraft): void;
  onRemove(): void;
}) {
  const type = properties.find((p) => p.key === rule.property)?.type;
  const kinds = kindsFor(type);
  return (
    <li className="flex flex-col gap-2 rounded-[4px] border border-border bg-bg p-2.5">
      <div className="flex items-center gap-2">
        <input
          type="checkbox"
          aria-label={`Use the ${kindInfo(rule.kind).label} rule on ${rule.property}`}
          checked={rule.enabled}
          onChange={(e) => onChange({ ...rule, enabled: e.target.checked })}
        />
        <select
          aria-label="Property"
          className={cn(inputClass, "h-7 flex-1")}
          value={rule.property}
          onChange={(e) => {
            const next = properties.find((p) => p.key === e.target.value);
            const allowed = kindsFor(next?.type);
            const kind = allowed.includes(rule.kind) ? rule.kind : (allowed[0] ?? rule.kind);
            onChange({ ...ruleDraft(e.target.value, kind, rule.enabled), key: rule.key });
          }}
        >
          {properties
            .filter((p) => kindsFor(p.type).length > 0)
            .map((p) => (
              <option key={p.key} value={p.key}>
                {p.key}
              </option>
            ))}
        </select>
        <button
          type="button"
          aria-label={`Remove the rule on ${rule.property}`}
          onClick={onRemove}
          className="text-text-3 hover:text-danger"
        >
          <Trash2 size={13} />
        </button>
      </div>
      <select
        aria-label="Rule kind"
        title={kindInfo(rule.kind).description}
        className={cn(inputClass, "h-7")}
        value={rule.kind}
        onChange={(e) =>
          onChange({
            ...ruleDraft(rule.property, e.target.value as RuleKind, rule.enabled),
            key: rule.key,
          })
        }
      >
        {kinds.map((kind) => (
          <option key={kind} value={kind}>
            {kindInfo(kind).label}
          </option>
        ))}
      </select>
      {rule.kind === "fuzzy_name" && (
        <label className="flex items-center gap-2 text-[11.5px] text-text-3">
          Threshold
          <input
            type="number"
            aria-label="Fuzzy threshold"
            min={RESOLUTION_LIMITS.thresholdMin}
            max={RESOLUTION_LIMITS.thresholdMax}
            step={0.01}
            className={cn(inputClass, "h-7 w-20")}
            value={rule.threshold ?? RESOLUTION_LIMITS.thresholdDefault}
            onChange={(e) => onChange({ ...rule, threshold: Number(e.target.value) })}
          />
          <span title="A heuristic cut-off, not a calibrated probability">heuristic</span>
        </label>
      )}
      {rule.kind === "normalized_phone" && (
        <label className="flex items-center gap-2 text-[11.5px] text-text-3">
          Default country
          <input
            aria-label="Default country"
            maxLength={2}
            className={cn(inputClass, "h-7 w-14 uppercase")}
            value={rule.country ?? ""}
            onChange={(e) => onChange({ ...rule, country: e.target.value.toUpperCase() })}
          />
          <span>for numbers without +code</span>
        </label>
      )}
      {rule.kind === "normalized_email" && (
        <label className="flex items-center gap-2 text-[11.5px] text-text-3">
          <input
            type="checkbox"
            checked={rule.case_insensitive_local_part ?? true}
            onChange={(e) => onChange({ ...rule, case_insensitive_local_part: e.target.checked })}
          />
          Count letter-case-only differences (weaker)
        </label>
      )}
    </li>
  );
}

function Coverage({ scan }: { scan: ResolutionScan }) {
  const reasons = [
    scan.truncated &&
      `only the first ${scan.entities_examined} ${scan.entity_type} entities were examined`,
    scan.comparison_budget_reached &&
      `the comparison budget (${scan.comparison_budget.toLocaleString()}) was reached`,
    scan.oversized_blocks > 0 && `${scan.oversized_blocks} oversized blocks were skipped`,
    scan.candidate_limit_reached && "lower-scoring candidates were dropped",
  ].filter(Boolean) as string[];
  const skipped = scan.skipped;
  return (
    <section
      aria-label="Scan coverage"
      className="flex flex-col gap-2 rounded-[6px] border border-border bg-bg p-3 text-[12px]"
    >
      <div className="flex items-center justify-between">
        <Kicker>Last scan · {scan.entity_type}</Kicker>
        {scan.exhaustive ? (
          <Badge tone="ok">Complete</Badge>
        ) : (
          <Badge tone="warn">Partial scan</Badge>
        )}
      </div>
      <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-text-2">
        <dt className="text-text-3">Entities examined</dt>
        <dd className="text-right font-mono text-text">
          {scan.entities_examined}
          {scan.truncated ? "+" : ""}
        </dd>
        <dt className="text-text-3">Pairs compared</dt>
        <dd className="text-right font-mono text-text">
          {scan.pairs_compared.toLocaleString()} / {scan.comparison_budget.toLocaleString()}
        </dd>
        <dt className="text-text-3">New candidates</dt>
        <dd className="text-right font-mono text-text">{scan.candidates_found}</dd>
        <dt className="text-text-3">Skipped pairs</dt>
        <dd
          className="text-right font-mono text-text"
          title={`${skipped.already_pending} already pending · ${skipped.previously_rejected} rejected earlier · ${skipped.already_linked} already linked`}
        >
          {skipped.already_pending + skipped.previously_rejected + skipped.already_linked}
        </dd>
      </dl>
      {reasons.length > 0 && <p className="text-warn">Not exhaustive: {reasons.join("; ")}.</p>}
      {scan.warnings.length > 0 && (
        <ul className="flex flex-col gap-0.5 text-text-3">
          {scan.warnings.map((warning) => (
            <li key={`${warning.code}-${warning.property}-${warning.rule}`}>
              <span className="font-mono">{warning.property}</span>: {warningText(warning)}
            </li>
          ))}
        </ul>
      )}
      <span className="text-text-3">
        {new Date(scan.created_at).toLocaleString()} · {scan.duration_ms} ms
      </span>
    </section>
  );
}

// --- queue -----------------------------------------------------------------------------

function label(labels: Record<string, Entity>, id: string): string {
  return labels[id] ? entityLabel(labels[id]) : shortId(id);
}

function QueueItem({
  candidate,
  labels,
  selected,
  onSelect,
}: {
  candidate: ResolutionCandidate;
  labels: Record<string, Entity>;
  selected: boolean;
  onSelect(): void;
}) {
  const strength = evidenceStrength(candidate.signals);
  return (
    <li>
      <button
        type="button"
        aria-current={selected || undefined}
        onClick={onSelect}
        className={cn(
          "flex w-full flex-col gap-1 border-b border-border px-4 py-3 text-left",
          selected ? "bg-accent/10" : "hover:bg-raised",
        )}
      >
        <span className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-[13px] text-text">
            {label(labels, candidate.entity_a_id)} <span className="text-text-3">vs</span>{" "}
            {label(labels, candidate.entity_b_id)}
          </span>
          <span
            className="font-mono text-[12px] text-text-2"
            title="Ranking score, not a probability"
          >
            {formatScore(candidate.score)}
          </span>
        </span>
        <span className="flex items-center gap-2">
          <Badge tone={strength === "strong" ? "ok" : strength === "moderate" ? "accent" : "warn"}>
            {strength}
          </Badge>
          <span className="truncate text-[11.5px] text-text-3">
            {matchedSummary(candidate.signals)}
          </span>
        </span>
      </button>
    </li>
  );
}

// --- workspace -------------------------------------------------------------------------

export interface EntityResolutionProps {
  graphId: string;
  /** Only candidates involving this entity (from the Graph Explorer). */
  initialEntityId?: string;
  /** The candidate to open first. */
  initialCandidateId?: string;
  api?: ApiClient;
}

interface Queue {
  key: string;
  page: CandidatePage;
  items: ResolutionCandidate[];
}

export function EntityResolution({
  graphId,
  initialEntityId,
  initialCandidateId,
  api: injected,
}: EntityResolutionProps) {
  const api = useMemo(() => injected ?? createApiClient(), [injected]);
  const [graph, setGraph] = useState<GraphProject | null>(null);
  const [ontology, setOntology] = useState<Ontology | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [entityType, setEntityType] = useState<string | null>(null);
  const [edited, setEdited] = useState<Record<string, RuleDraft[]>>({});
  const [maxEntities, setMaxEntities] = useState<number>(RESOLUTION_LIMITS.maxEntitiesDefault);
  const [scan, setScan] = useState<ResolutionScan | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [tab, setTab] = useState<CandidateStatus>("pending");
  const [entityFilter, setEntityFilter] = useState<string | null>(initialEntityId ?? null);
  const [refresh, setRefresh] = useState(0);
  const [queue, setQueue] = useState<Queue | null>(null);
  const [queueError, setQueueError] = useState<string | null>(null);
  const [labels, setLabels] = useState<Record<string, Entity>>({});
  const [selected, setSelected] = useState<string | null>(initialCandidateId ?? null);
  const [detail, setDetail] = useState<CandidateDetail | null>(null);
  const [detailError, setDetailError] = useState<{ id: string; message: string } | null>(null);
  const [deciding, setDeciding] = useState(false);
  const [decisionError, setDecisionError] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);

  // Graph, ontology and the latest scan; state changes only in callbacks.
  useEffect(() => {
    let cancelled = false;
    Promise.all([
      api.getGraph(graphId),
      api.getOntology(graphId),
      api.listResolutionScans(graphId, 1),
    ]).then(
      ([foundGraph, foundOntology, scans]) => {
        if (cancelled) return;
        setGraph(foundGraph);
        setOntology(foundOntology);
        setScan(scans[0] ?? null);
      },
      (error: unknown) => {
        if (!cancelled) setLoadError(message(error));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [api, graphId]);

  const types = useMemo(() => ontology?.entity_types ?? [], [ontology]);
  const suggested = useMemo(
    () =>
      Object.fromEntries(
        types.map((type) => [
          type.name,
          suggestRules(type.properties.map((p) => ({ key: p.key, type: p.type }))),
        ]),
      ) as Record<string, RuleDraft[]>,
    [types],
  );
  const activeType =
    entityType ??
    scan?.entity_type ??
    types.find((type) => suggested[type.name]?.some((rule) => rule.enabled))?.name ??
    types[0]?.name ??
    null;
  const activeProperties = types.find((type) => type.name === activeType)?.properties ?? [];
  const rules = activeType ? (edited[activeType] ?? suggested[activeType] ?? []) : [];
  const setRules = (next: RuleDraft[]) =>
    activeType && setEdited({ ...edited, [activeType]: next });
  const request = rulesToRequest(rules);

  // The review queue for the current tab and filter.
  const queueKey = `${tab}|${entityFilter ?? ""}|${refresh}`;
  useEffect(() => {
    let cancelled = false;
    const key = `${tab}|${entityFilter ?? ""}|${refresh}`;
    api
      .listResolutionCandidates(graphId, {
        status: tab,
        entity_id: entityFilter ?? undefined,
        limit: RESOLUTION_LIMITS.pageDefault,
      })
      .then(
        async (page) => {
          if (cancelled) return;
          setQueue({ key, page, items: page.data });
          setQueueError(null);
          const ids = [...new Set(page.data.flatMap((c) => [c.entity_a_id, c.entity_b_id]))];
          if (ids.length === 0) return;
          try {
            const subgraph = await api.getSubgraph(graphId, ids);
            if (cancelled) return;
            setLabels((known) => {
              const next = { ...known };
              for (const entity of subgraph.entities) next[entity.id] = entity;
              return next;
            });
          } catch {
            // Labels fall back to short ids.
          }
        },
        (error: unknown) => {
          if (!cancelled) setQueueError(message(error));
        },
      );
    return () => {
      cancelled = true;
    };
  }, [api, graphId, tab, entityFilter, refresh]);

  const current = queue?.key === queueKey ? queue : null;
  const items = current?.items ?? [];
  const effectiveId = selected ?? items[0]?.id ?? null;

  // The selected candidate with both original entities.
  useEffect(() => {
    if (!effectiveId) return;
    let cancelled = false;
    api.getResolutionCandidate(graphId, effectiveId).then(
      (found) => {
        if (cancelled) return;
        setDetail(found);
        setDetailError(null);
        setLabels((known) => {
          const next = { ...known };
          for (const entity of [found.entity_a, found.entity_b]) {
            if (entity) next[entity.id] = entity;
          }
          return next;
        });
      },
      (error: unknown) => {
        if (!cancelled) setDetailError({ id: effectiveId, message: message(error) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [api, graphId, effectiveId, refresh]);

  const shownDetail = detail && detail.candidate.id === effectiveId ? detail : null;

  const runScan = async () => {
    if (!activeType) return;
    setScanning(true);
    setScanError(null);
    try {
      const result = await api.runResolutionScan(graphId, {
        entity_type: activeType,
        max_entities: maxEntities,
        rules: request,
      });
      setScan(result);
      setTab("pending");
      setSelected(null);
      setRefresh((n) => n + 1);
    } catch (error) {
      setScanError(message(error));
    } finally {
      setScanning(false);
    }
  };

  const decide = async (decision: DecisionRequest) => {
    if (!shownDetail) return;
    const id = shownDetail.candidate.id;
    setDeciding(true);
    setDecisionError(null);
    try {
      await api.decideResolutionCandidate(graphId, id, decision);
      setSelected(id);
    } catch (error) {
      setDecisionError(
        error instanceof ApiError && error.status === 409
          ? `${error.message}. The candidate was reloaded with its current state.`
          : message(error),
      );
    } finally {
      setDeciding(false);
      setRefresh((n) => n + 1);
    }
  };

  const nextPending = items.find((c) => c.id !== effectiveId && c.status === "pending");

  const loadMore = async () => {
    if (!current?.page.next_cursor) return;
    setLoadingMore(true);
    try {
      const page = await api.listResolutionCandidates(graphId, {
        status: tab,
        entity_id: entityFilter ?? undefined,
        limit: RESOLUTION_LIMITS.pageDefault,
        cursor: current.page.next_cursor,
      });
      setQueue({ key: current.key, page, items: [...current.items, ...page.data] });
    } catch (error) {
      setQueueError(message(error));
    } finally {
      setLoadingMore(false);
    }
  };

  const counts = current?.page.counts;

  return (
    <AppShell graphId={graphId} graphName={graph?.name}>
      <div className="flex h-screen min-h-0 flex-col">
        <header className="flex flex-wrap items-end justify-between gap-4 border-b border-border px-8 py-5">
          <div className="flex flex-col gap-1.5">
            <Kicker className="text-accent">Entity resolution</Kicker>
            <h1 className="text-[24px] leading-tight font-bold text-text">Potential duplicates</h1>
            <p className="max-w-[720px] text-[13px] text-text-2">
              Find records that may describe the same real-world entity and review them. Approving
              creates a logical identity mapping; original entities and relationships stay
              unchanged.
            </p>
          </div>
          <Link
            href={`/graphs/${graphId}/explore`}
            className="text-[12.5px] text-text-2 hover:text-text"
          >
            Open Graph Explorer
          </Link>
        </header>

        {loadError && (
          <div className="px-8 pt-4">
            <ErrorNote>{loadError}</ErrorNote>
          </div>
        )}

        <div className="grid min-h-0 flex-1 grid-cols-[280px_300px_minmax(0,1fr)]">
          <aside
            aria-label="Resolution rules"
            className="flex min-h-0 flex-col gap-4 overflow-y-auto border-r border-border bg-panel px-4 py-4"
          >
            <Field label="Entity type">
              <select
                className={inputClass}
                value={activeType ?? ""}
                onChange={(e) => setEntityType(e.target.value)}
                disabled={types.length === 0}
              >
                {types.map((type) => (
                  <option key={type.name} value={type.name}>
                    {type.display_name || type.name}
                  </option>
                ))}
              </select>
            </Field>
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <Kicker>Matching rules</Kicker>
                <span className="font-mono text-[10px] text-text-3">
                  {request.length} / {RESOLUTION_LIMITS.maxRules}
                </span>
              </div>
              {rules.length === 0 && (
                <p className="text-[12px] text-text-3">
                  No rule yet. Add one for a property that can identify a record.
                </p>
              )}
              <ul className="flex flex-col gap-2">
                {rules.map((rule) => (
                  <RuleRow
                    key={rule.key}
                    rule={rule}
                    properties={activeProperties}
                    onChange={(next) => setRules(rules.map((r) => (r.key === rule.key ? next : r)))}
                    onRemove={() => setRules(rules.filter((r) => r.key !== rule.key))}
                  />
                ))}
              </ul>
              <Button
                size="sm"
                icon={<Plus size={13} />}
                disabled={rules.length >= RESOLUTION_LIMITS.maxRules}
                onClick={() => {
                  const property =
                    activeProperties.find((p) => kindsFor(p.type).includes("normalized_string")) ??
                    activeProperties.find((p) => kindsFor(p.type).length > 0);
                  if (property) {
                    setRules([...rules, ruleDraft(property.key, kindsFor(property.type)[0])]);
                  }
                }}
              >
                Add rule
              </Button>
            </div>
            <Field
              label={`Entities to examine (${RESOLUTION_LIMITS.maxEntitiesMin} – ${RESOLUTION_LIMITS.maxEntitiesMax})`}
            >
              <input
                type="number"
                className={inputClass}
                min={RESOLUTION_LIMITS.maxEntitiesMin}
                max={RESOLUTION_LIMITS.maxEntitiesMax}
                value={maxEntities}
                onChange={(e) => setMaxEntities(Math.round(Number(e.target.value)))}
              />
            </Field>
            <Button
              variant="primary"
              size="lg"
              disabled={scanning || !activeType || request.length === 0}
              icon={
                scanning ? <LoaderCircle size={15} className="animate-spin" /> : <Play size={15} />
              }
              onClick={() => void runScan()}
            >
              {scanning ? "Scanning…" : "Run Scan"}
            </Button>
            {scanError && <ErrorNote>{scanError}</ErrorNote>}
            {scan && <Coverage scan={scan} />}
            <p className="flex gap-1.5 text-[11px] text-text-3">
              <Info size={12} className="mt-0.5 shrink-0" />
              Scans are bounded: at most {RESOLUTION_LIMITS.maxEntitiesMax} entities and{" "}
              {(20_000).toLocaleString()} comparisons. Rules: {RULE_KINDS.length} kinds, no AI.
            </p>
          </aside>

          <section
            aria-label="Candidates"
            className="flex min-h-0 flex-col border-r border-border bg-bg"
          >
            <div
              role="tablist"
              aria-label="Candidate status"
              className="flex border-b border-border"
            >
              {TABS.map(({ status, label: name }) => (
                <button
                  key={status}
                  type="button"
                  role="tab"
                  aria-selected={tab === status}
                  onClick={() => {
                    setTab(status);
                    setSelected(null);
                  }}
                  className={cn(
                    "flex-1 border-b-2 px-3 py-2.5 text-[12.5px]",
                    tab === status
                      ? "border-accent text-text"
                      : "border-transparent text-text-3 hover:text-text-2",
                  )}
                >
                  {name}
                  {counts && (
                    <span className="ml-1.5 font-mono text-[11px] text-text-3">
                      {counts[status]}
                    </span>
                  )}
                </button>
              ))}
            </div>
            {entityFilter && (
              <div className="flex items-center gap-2 border-b border-border px-4 py-2 text-[12px] text-text-2">
                <CopyCheck size={13} className="text-warn" />
                <span className="min-w-0 flex-1 truncate">
                  Involving {label(labels, entityFilter)}
                </span>
                <button
                  type="button"
                  aria-label="Show all candidates"
                  onClick={() => {
                    setEntityFilter(null);
                    setSelected(null);
                  }}
                  className="text-text-3 hover:text-text"
                >
                  <X size={13} />
                </button>
              </div>
            )}
            <div className="min-h-0 flex-1 overflow-y-auto">
              {queueError && (
                <div className="p-4">
                  <ErrorNote>{queueError}</ErrorNote>
                </div>
              )}
              {!current && !queueError && (
                <p className="p-4 text-[12.5px] text-text-3">Loading candidates…</p>
              )}
              {current && items.length === 0 && (
                <p className="p-4 text-[12.5px] text-text-3">
                  {tab === "pending"
                    ? "No pending candidates. Configure rules and run a scan."
                    : `No ${tab} candidates yet.`}
                </p>
              )}
              <ul>
                {items.map((candidate) => (
                  <QueueItem
                    key={candidate.id}
                    candidate={candidate}
                    labels={labels}
                    selected={candidate.id === effectiveId}
                    onSelect={() => setSelected(candidate.id)}
                  />
                ))}
              </ul>
              {current?.page.next_cursor && (
                <div className="p-3">
                  <Button
                    size="sm"
                    className="w-full"
                    disabled={loadingMore}
                    onClick={() => void loadMore()}
                  >
                    {loadingMore ? "Loading…" : "Load more"}
                  </Button>
                </div>
              )}
            </div>
          </section>

          <section aria-label="Review" className="flex min-h-0 flex-col bg-bg">
            {shownDetail ? (
              <CandidateReview
                key={`${shownDetail.candidate.id}:${shownDetail.candidate.status}:${shownDetail.decisions.length}`}
                graphId={graphId}
                detail={shownDetail}
                deciding={deciding}
                error={decisionError}
                onDecide={(decision) => void decide(decision)}
                onNext={
                  nextPending
                    ? () => {
                        setDecisionError(null);
                        setSelected(nextPending.id);
                      }
                    : undefined
                }
              />
            ) : detailError && detailError.id === effectiveId ? (
              <div className="p-6">
                <ErrorNote>{detailError.message}</ErrorNote>
              </div>
            ) : (
              <p className="p-6 text-[13px] text-text-3">
                {effectiveId ? "Loading candidate…" : "Select a candidate to compare both records."}
              </p>
            )}
          </section>
        </div>

        <footer
          aria-label="Review status"
          className="flex h-8 shrink-0 items-center gap-5 border-t border-border bg-panel px-6 font-mono text-[11px] text-text-3"
        >
          <span>
            Pending: <span className="text-text">{counts?.pending ?? "…"}</span>
          </span>
          <span>
            Accepted: <span className="text-ok">{counts?.accepted ?? "…"}</span>
          </span>
          <span>
            Rejected: <span className="text-text-2">{counts?.rejected ?? "…"}</span>
          </span>
          <span className="ml-auto">Logical identity only · graph unchanged</span>
        </footer>
      </div>
    </AppShell>
  );
}
