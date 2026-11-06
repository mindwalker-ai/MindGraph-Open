"use client";

/**
 * The review side of the Entity Resolution workspace: both original
 * entities side by side, the evidence per property, and the decision.
 *
 * "Same entity" records a logical identity mapping and asks which original
 * entity stays canonical. Nothing here changes the graph, so there is no
 * destructive-merge confirmation; the panel says so instead.
 */

import {
  ArrowRight,
  Check,
  CircleSlash,
  ExternalLink,
  GitMerge,
  History,
  LoaderCircle,
  RotateCcw,
  ShieldCheck,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";

import { Badge, Button, ErrorNote, Kicker } from "@/components/ui/primitives";
import type {
  CandidateDetail,
  DecisionRequest,
  Entity,
  EntityIdentity,
  MatchSignal,
  PropertyJson,
  ResolutionDecision,
  SignalOutcome,
} from "@/lib/api/types";
import { typeStyle } from "@/lib/graph/palette";
import { entityLabel, formatValue, shortId } from "@/lib/graph/transform";
import {
  NOTE_LABEL,
  OUTCOME_LABEL,
  compareRows,
  evidenceStrength,
  formatScore,
  matchedSummary,
} from "@/lib/resolution/rules";
import { cn } from "@/lib/utils";

const OUTCOME_TONE: Record<SignalOutcome, "ok" | "accent" | "danger" | "neutral" | "warn"> = {
  exact: "ok",
  normalized: "ok",
  fuzzy: "accent",
  no_match: "danger",
  missing: "neutral",
  unavailable: "warn",
};

const STRENGTH_TONE = { strong: "ok", moderate: "accent", weak: "warn" } as const;

const STATUS_TONE = { accepted: "ok", rejected: "danger", pending: "neutral" } as const;

/** One rule's outcome for one property, with its similarity and note. */
export function SignalBadge({ signal }: { signal: MatchSignal }) {
  const similarity =
    signal.similarity !== undefined && (signal.outcome === "fuzzy" || signal.outcome === "no_match")
      ? ` ${formatScore(signal.similarity)}`
      : "";
  return (
    <span className="inline-flex flex-col items-start gap-0.5">
      <Badge tone={OUTCOME_TONE[signal.outcome]}>
        {OUTCOME_LABEL[signal.outcome]}
        {similarity}
      </Badge>
      {signal.note && <span className="text-[10.5px] text-text-3">{NOTE_LABEL[signal.note]}</span>}
    </span>
  );
}

function EntityHeading({
  entity,
  id,
  side,
  graphId,
}: {
  entity: Entity | null;
  id: string;
  side: "A" | "B";
  graphId: string;
}) {
  const color = entity ? typeStyle(entity.entity_type).cssColor : "var(--text-3)";
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div className="flex items-center gap-2">
        <span
          aria-hidden
          className="h-2.5 w-2.5 shrink-0 rounded-full"
          style={{ boxShadow: `inset 0 0 0 2px ${color}` }}
        />
        <Kicker>
          {side} · {entity?.entity_type ?? "Missing"}
        </Kicker>
      </div>
      <span className="truncate text-[16px] font-bold text-text">
        {entity ? entityLabel(entity) : "Entity no longer in the graph"}
      </span>
      <span className="flex items-center gap-2 font-mono text-[11px] text-text-3">
        {shortId(id)}
        {entity && (
          <Link
            href={`/graphs/${graphId}/explore?entity=${id}`}
            className="inline-flex items-center gap-1 text-text-3 hover:text-text"
            title="Open this original entity in the Graph Explorer"
          >
            <ExternalLink size={11} /> Explorer
          </Link>
        )}
      </span>
    </div>
  );
}

function ValueCell({ value, matched }: { value: PropertyJson | undefined; matched: boolean }) {
  if (value === undefined) return <span className="text-[12px] text-text-3 italic">—</span>;
  return (
    <span
      className={cn("font-mono text-[12px] break-words", matched ? "text-text" : "text-text-2")}
    >
      {formatValue(value)}
    </span>
  );
}

function IdentityLine({
  identity,
  labels,
}: {
  identity: EntityIdentity;
  labels: Record<string, Entity>;
}) {
  if (!identity.resolved) return <span className="text-text-3">Not linked yet</span>;
  if (identity.is_canonical) {
    return (
      <span>
        Canonical of{" "}
        <span className="text-text">{identity.member_entity_ids.length} linked records</span>
      </span>
    );
  }
  const canonical = labels[identity.canonical_entity_id];
  return (
    <span>
      Alias of{" "}
      <span className="text-text">
        {canonical ? entityLabel(canonical) : shortId(identity.canonical_entity_id)}
      </span>
    </span>
  );
}

function DecisionHistory({
  decisions,
  labels,
}: {
  decisions: ResolutionDecision[];
  labels: Record<string, Entity>;
}) {
  if (decisions.length === 0) return null;
  return (
    <section aria-label="Decision history" className="flex flex-col gap-2">
      <Kicker className="flex items-center gap-1.5">
        <History size={11} /> Decision history
      </Kicker>
      <ol className="flex flex-col gap-1.5">
        {decisions.map((decision) => {
          const canonical = decision.canonical_entity_id;
          return (
            <li
              key={decision.id}
              className="flex flex-wrap items-center gap-x-2 gap-y-0.5 rounded-[4px] border border-border bg-bg px-3 py-2 text-[12px] text-text-2"
            >
              <Badge tone={decision.decision === "same_entity" ? "ok" : "neutral"}>
                {decision.decision === "same_entity" ? "Same entity" : "Different entities"}
              </Badge>
              {decision.reconsidered && <Badge tone="warn">Reconsidered</Badge>}
              {canonical && (
                <span>
                  canonical{" "}
                  <span className="text-text">
                    {labels[canonical] ? entityLabel(labels[canonical]) : shortId(canonical)}
                  </span>
                </span>
              )}
              <span className="text-text-3">
                · score {formatScore(decision.score)} ·{" "}
                {decision.matched_rules.join(", ") || "no agreeing rules"} · {decision.actor} ·{" "}
                {new Date(decision.created_at).toLocaleString()}
              </span>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

export interface CandidateReviewProps {
  graphId: string;
  detail: CandidateDetail;
  deciding: boolean;
  error: string | null;
  onDecide(request: DecisionRequest): void;
  /** Selects the next pending candidate, when there is one. */
  onNext?: () => void;
}

export function CandidateReview({
  graphId,
  detail,
  deciding,
  error,
  onDecide,
  onNext,
}: CandidateReviewProps) {
  const { candidate, entity_a: a, entity_b: b } = detail;
  const [chooser, setChooser] = useState<"same" | "reconsider" | null>(null);
  const [canonical, setCanonical] = useState<string | null>(null);
  const labels: Record<string, Entity> = {};
  if (a) labels[a.id] = a;
  if (b) labels[b.id] = b;
  const rows = compareRows(candidate.signals, a, b);
  const strength = evidenceStrength(candidate.signals);
  // The canonical entity may be any member of the resulting group.
  const options = [
    ...new Set([
      candidate.entity_a_id,
      candidate.entity_b_id,
      ...detail.identity_a.member_entity_ids,
      ...detail.identity_b.member_entity_ids,
    ]),
  ];
  const chosen = canonical && options.includes(canonical) ? canonical : null;
  const pending = candidate.status === "pending";
  const missing = !a || !b;
  // Both records already share one identity through other decisions.
  const linked =
    detail.identity_a.resolved &&
    detail.identity_a.canonical_entity_id === detail.identity_b.canonical_entity_id;
  // Two equal columns (record A, record B), readable in a narrow review pane.
  const grid = "grid grid-cols-2 gap-x-4";

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-6 py-5">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <Kicker>Potential duplicate · {candidate.entity_type}</Kicker>
          <h2 className="text-[20px] font-bold break-words text-text">
            {a ? entityLabel(a) : shortId(candidate.entity_a_id)}
            <span className="px-2 font-normal text-text-3">vs</span>
            {b ? entityLabel(b) : shortId(candidate.entity_b_id)}
          </h2>
          <p className="text-[12.5px] text-text-2">{matchedSummary(candidate.signals)}</p>
        </div>
        <div className="flex flex-col items-end gap-1">
          <span
            className="font-mono text-[18px] font-semibold text-text"
            title="A ranking heuristic that orders review work. It is not a probability that the records are the same."
          >
            score {formatScore(candidate.score)}
          </span>
          <span className="flex items-center gap-1.5">
            <Badge tone={STRENGTH_TONE[strength]}>{strength} evidence</Badge>
            <Badge tone={STATUS_TONE[candidate.status]}>{candidate.status}</Badge>
          </span>
          <span className="text-[10.5px] text-text-3">ranking heuristic, not a probability</span>
        </div>
      </header>

      <section
        aria-label="Compare entities"
        className="rounded-[6px] border border-border bg-panel"
      >
        <div className={cn(grid, "border-b border-border px-4 py-3")}>
          <EntityHeading entity={a} id={candidate.entity_a_id} side="A" graphId={graphId} />
          <EntityHeading entity={b} id={candidate.entity_b_id} side="B" graphId={graphId} />
        </div>
        {rows.map((row) => {
          const matched = row.signals.some((signal) => signal.matched);
          return (
            <div
              key={row.key}
              data-matched={matched || undefined}
              className={cn(
                "flex flex-col gap-1.5 border-b border-border px-4 py-2.5 last:border-b-0",
                matched && "bg-ok/5",
              )}
            >
              <div className="flex flex-wrap items-start justify-between gap-2">
                <span className="font-mono text-[11px] text-text-3">{row.key}</span>
                <span className="flex flex-wrap items-start justify-end gap-2">
                  {row.signals.length === 0 ? (
                    <span className="text-[11px] text-text-3">not compared</span>
                  ) : (
                    row.signals.map((signal) => (
                      <SignalBadge key={`${signal.rule}-${signal.property}`} signal={signal} />
                    ))
                  )}
                </span>
              </div>
              <div className={grid}>
                <ValueCell value={row.a} matched={matched} />
                <ValueCell value={row.b} matched={matched} />
              </div>
            </div>
          );
        })}
        <div className={cn(grid, "border-t border-border px-4 py-2 text-[12px] text-text-2")}>
          <IdentityLine identity={detail.identity_a} labels={labels} />
          <IdentityLine identity={detail.identity_b} labels={labels} />
        </div>
      </section>

      {error && <ErrorNote>{error}</ErrorNote>}
      {missing && (
        <ErrorNote>
          An entity of this candidate is no longer in the graph, so it cannot be approved.
        </ErrorNote>
      )}
      {linked && pending && (
        <p className="rounded-[4px] border border-border bg-raised px-3 py-2 text-[12.5px] text-text-2">
          These records are already linked through other decisions. Approving records this pair
          without changing the group; splitting a group is not supported in v0.1.
        </p>
      )}

      {chooser ? (
        <section
          aria-label="Choose the canonical entity"
          className="flex flex-col gap-3 rounded-[6px] border border-accent/50 bg-accent/5 p-4"
        >
          <p className="flex items-start gap-2 text-[12.5px] text-text-2">
            <ShieldCheck size={15} className="mt-0.5 shrink-0 text-ok" />
            This creates a logical identity mapping. Original graph entities and relationships will
            remain unchanged.
          </p>
          <fieldset className="flex flex-col gap-2">
            <legend className="mb-1 text-[12px] text-text-3">
              Which original entity stays canonical?
            </legend>
            {options.map((id) => {
              const entity = labels[id];
              return (
                <label
                  key={id}
                  className={cn(
                    "flex cursor-pointer items-center gap-3 rounded-[4px] border px-3 py-2 text-[13px]",
                    chosen === id ? "border-accent bg-accent/10" : "border-border bg-bg",
                  )}
                >
                  <input
                    type="radio"
                    name="canonical"
                    value={id}
                    checked={chosen === id}
                    onChange={() => setCanonical(id)}
                  />
                  <span className="text-text">{entity ? entityLabel(entity) : shortId(id)}</span>
                  <span className="font-mono text-[11px] text-text-3">{shortId(id)}</span>
                </label>
              );
            })}
          </fieldset>
          <div className="flex gap-2">
            <Button
              variant="primary"
              disabled={!chosen || deciding}
              icon={
                deciding ? <LoaderCircle size={14} className="animate-spin" /> : <Check size={14} />
              }
              onClick={() =>
                chosen &&
                onDecide({
                  decision: "same_entity",
                  canonical_entity_id: chosen,
                  ...(chooser === "reconsider" ? { reconsider: true } : {}),
                })
              }
            >
              Confirm same entity
            </Button>
            <Button onClick={() => setChooser(null)} disabled={deciding}>
              Cancel
            </Button>
          </div>
        </section>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          {pending && (
            <>
              <Button
                variant="primary"
                icon={<GitMerge size={14} />}
                disabled={deciding || missing}
                onClick={() => setChooser("same")}
              >
                Same Entity
              </Button>
              <Button
                icon={
                  deciding ? (
                    <LoaderCircle size={14} className="animate-spin" />
                  ) : (
                    <CircleSlash size={14} />
                  )
                }
                disabled={deciding || linked}
                onClick={() => onDecide({ decision: "different_entities" })}
              >
                Different Entities
              </Button>
            </>
          )}
          {candidate.status === "rejected" && (
            <Button
              icon={<RotateCcw size={14} />}
              disabled={deciding || missing}
              onClick={() => setChooser("reconsider")}
              title="Review this rejected pair again as the same entity"
            >
              Reconsider as same entity
            </Button>
          )}
          {candidate.status === "accepted" && (
            <p className="text-[12.5px] text-text-2">
              Linked as one logical identity. The original entities stay separate in the graph.
            </p>
          )}
          {onNext && (
            <Button variant="ghost" icon={<ArrowRight size={14} />} onClick={onNext}>
              Next Candidate
            </Button>
          )}
        </div>
      )}

      <DecisionHistory decisions={detail.decisions} labels={labels} />
    </div>
  );
}
