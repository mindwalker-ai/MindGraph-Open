"use client";

/**
 * Ask MindGraph (mockup 08): a contextual investigation assistant in the
 * explorer's right panel, never a full-screen chat. The graph stays
 * visible; answers come with the evidence MindGraph Core retrieved, and
 * every reference can be shown on the graph.
 *
 * The API answers synchronously, so the panel shows one honest "answering"
 * state instead of invented phases or token streaming.
 *
 * Follow-ups: the panel keeps the conversation (in the browser only) and
 * sends the last three questions with the entities they cited, so "they"
 * or "what about C003?" can be understood. Evidence is retrieved afresh for
 * every question; "New conversation" forgets the earlier ones.
 */

import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Copy,
  Eraser,
  History,
  ListTree,
  LoaderCircle,
  MessageSquareText,
  ScanSearch,
  TriangleAlert,
  X,
} from "lucide-react";
import { type KeyboardEvent, useEffect, useRef, useState } from "react";

import { Badge, Button, ErrorNote, Kicker } from "@/components/ui/primitives";
import {
  ASK_LIMITS,
  type AnswerStatus,
  type AskHistoryTurn,
  type AskResponse,
} from "@/lib/api/types";
import { type Suggestion, citationTokens, relationshipTypeCounts } from "@/lib/graph/evidence";
import { typeStyle } from "@/lib/graph/palette";
import { entityLabel } from "@/lib/graph/transform";
import { cn } from "@/lib/utils";

export type AskAvailability = "checking" | "ready" | "not_configured" | "unavailable";

/** An answered question of the conversation. */
export interface AskTurn {
  question: string;
  scope: string;
  answer: AskResponse;
  /** What a follow-up sends about this question. */
  turn: AskHistoryTurn;
}

export type AskRun =
  | { status: "idle" }
  | { status: "answering"; question: string; scope: string }
  | ({ status: "completed" } & AskTurn)
  | { status: "error"; question: string; scope: string; message: string };

export interface AskPanelProps {
  availability: AskAvailability;
  /** The selected entity, which scopes the question when `scope` is "selected". */
  selected: { label: string; entityType: string } | null;
  scope: "selected" | "graph";
  question: string;
  /** Earlier answered questions of the conversation, oldest first. */
  thread: AskTurn[];
  run: AskRun;
  suggestions: Suggestion[];
  /** The answer's evidence is highlighted on the graph. */
  evidenceShown: boolean;
  onScope(scope: "selected" | "graph"): void;
  onQuestion(question: string): void;
  onSubmit(question?: string): void;
  onShowOnGraph(): void;
  onShowRef(ref: string): void;
  onClearEvidence(): void;
  onOpenEntity(entityId: string): void;
  /** Shows the evidence of an earlier question (`thread[index]`). */
  onShowTurn(index: number): void;
  /** Forgets the conversation. */
  onNewConversation(): void;
  onClose(): void;
}

const STATUS: Record<AnswerStatus, { label: string; tone: "ok" | "warn" | "danger" }> = {
  answered: { label: "Answered", tone: "ok" },
  partial: { label: "Partial", tone: "warn" },
  insufficient_evidence: { label: "Insufficient evidence", tone: "danger" },
};

const OPERATION: Record<string, string> = {
  entity_lookup: "entity lookup",
  neighbors: "neighbourhood",
  find_path: "shortest path",
  shared_neighbor: "shared neighbour",
  unsupported: "unsupported",
};

function RefChip({ reference, onShow }: { reference: string; onShow(ref: string): void }) {
  return (
    <button
      type="button"
      onClick={() => onShow(reference)}
      title={`Show ${reference} on the graph`}
      className="mx-0.5 inline-flex items-center rounded-[3px] border border-risk/50 bg-risk/10 px-1 font-mono text-[10px] leading-[16px] text-risk hover:bg-risk/20"
    >
      {reference}
    </button>
  );
}

function AnswerText({ answer, onShowRef }: { answer: AskResponse; onShowRef(ref: string): void }) {
  const known = new Set([
    ...answer.evidence.entities.map((e) => e.ref),
    ...answer.evidence.relationships.map((r) => r.ref),
    ...answer.evidence.paths.map((p) => p.ref),
  ]);
  return (
    <p className="text-[13.5px] leading-[1.55] whitespace-pre-wrap">
      {citationTokens(answer.answer, known).map((token, index) =>
        "text" in token ? (
          <span key={index}>{token.text}</span>
        ) : (
          <span key={index} className="whitespace-nowrap">
            {token.refs.map((ref) => (
              <RefChip key={ref} reference={ref} onShow={onShowRef} />
            ))}
          </span>
        ),
      )}
    </p>
  );
}

/** Supporting paths: entities in order with the relationships between them. */
function Paths({
  answer,
  onShowRef,
  onOpenEntity,
}: {
  answer: AskResponse;
  onShowRef(ref: string): void;
  onOpenEntity(id: string): void;
}) {
  const [all, setAll] = useState(false);
  const { paths, entities, relationships } = answer.evidence;
  if (paths.length === 0) return null;
  const shown = all ? paths : paths.slice(0, 2);
  const byId = new Map(entities.map((e) => [e.id, e]));
  const relById = new Map(relationships.map((r) => [r.id, r]));
  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <Kicker>Supporting path{paths.length === 1 ? "" : "s"}</Kicker>
        <span className="font-mono text-[10px] text-text-3">
          {shown.length} of {paths.length}
        </span>
      </div>
      {shown.map((path) => (
        <div key={path.ref} className="flex flex-col">
          <button
            type="button"
            onClick={() => onShowRef(path.ref)}
            className="mb-1 self-start font-mono text-[10px] text-risk hover:underline"
          >
            {path.ref} · show path
          </button>
          {path.entity_ids.map((id, index) => {
            const entity = byId.get(id);
            const relationship = relById.get(path.relationship_ids[index] ?? "");
            if (!entity) return null;
            const forward = relationship?.source_id === id;
            const middle = index > 0 && index < path.entity_ids.length - 1;
            return (
              <div key={`${path.ref}-${id}-${index}`} className="flex flex-col">
                <button
                  type="button"
                  onClick={() => onOpenEntity(id)}
                  className={cn(
                    "flex items-center gap-2 rounded-[4px] border px-2.5 py-1.5 text-left",
                    middle
                      ? "border-risk/50 bg-risk/10 hover:bg-risk/15"
                      : "border-border bg-raised hover:bg-hover",
                  )}
                >
                  <span
                    aria-hidden
                    className="h-2.5 w-2.5 shrink-0 rounded-full"
                    style={{ background: typeStyle(entity.entity_type).cssColor }}
                  />
                  <span className="min-w-0 flex-1 truncate text-[12.5px]">
                    {entityLabel(entity)}
                  </span>
                  <span className="font-mono text-[10px] text-text-3">
                    {entity.ref} · {entity.entity_type}
                  </span>
                </button>
                {relationship && (
                  <span className="ml-[13px] flex items-center gap-1.5 border-l border-risk/60 py-1 pl-3 font-mono text-[10px] text-risk">
                    {forward ? <ArrowRight size={11} /> : <ArrowLeft size={11} />}
                    {relationship.relationship_type}
                  </span>
                )}
              </div>
            );
          })}
        </div>
      ))}
      {paths.length > 2 && (
        <button
          type="button"
          onClick={() => setAll((value) => !value)}
          className="self-start text-[11.5px] text-text-3 hover:text-text"
        >
          {all ? "Show fewer paths" : `Show all ${paths.length} paths`}
        </button>
      )}
    </section>
  );
}

function Evidence({ answer, onShowRef }: { answer: AskResponse; onShowRef(ref: string): void }) {
  const { entities } = answer.evidence;
  if (entities.length === 0) return null;
  return (
    <section className="flex flex-col gap-2">
      <Kicker>
        Evidence · {entities.length} {entities.length === 1 ? "entity" : "entities"} ·{" "}
        {answer.evidence.relationships.length} relationships
      </Kicker>
      <div className="flex flex-wrap gap-1">
        {relationshipTypeCounts(answer).map(({ type, count }) => (
          <span
            key={type}
            className="rounded-[3px] border border-border-strong px-1.5 py-px font-mono text-[10px] text-text-2"
          >
            {type} ×{count}
          </span>
        ))}
      </div>
      <ul aria-label="Evidence entities" className="flex flex-col">
        {entities.slice(0, 12).map((entity) => (
          <li key={entity.id}>
            <button
              type="button"
              onClick={() => onShowRef(entity.ref)}
              className="flex w-full items-center gap-2 rounded-[4px] px-1.5 py-1 text-left hover:bg-raised"
            >
              <span className="w-7 font-mono text-[10px] text-risk">{entity.ref}</span>
              <span
                aria-hidden
                className="h-2 w-2 shrink-0 rounded-full"
                style={{ background: typeStyle(entity.entity_type).cssColor }}
              />
              <span className="min-w-0 flex-1 truncate text-[12.5px]">{entityLabel(entity)}</span>
              <span className="text-[10.5px] text-text-3">{entity.entity_type}</span>
            </button>
          </li>
        ))}
      </ul>
      {entities.length > 12 && (
        <p className="text-[11px] text-text-3">
          +{entities.length - 12} more; all of them are highlighted with Show on Graph.
        </p>
      )}
    </section>
  );
}

function Plan({ answer }: { answer: AskResponse }) {
  const { plan, operations, candidates_inspected, context_bytes } = answer.retrieval;
  const rows: [string, string | number | undefined][] = [
    ["operation", OPERATION[plan.operation] ?? plan.operation],
    ["from", plan.source],
    ["to", plan.target],
    ["entity type", plan.entity_type],
    ["shared type", plan.shared_entity_type],
    ["relationship", plan.relationship_type],
    ["direction", plan.direction],
    ["max depth", plan.max_depth],
    ["graph operations", operations],
    ["candidates inspected", candidates_inspected],
    ["context sent", context_bytes ? `${(context_bytes / 1024).toFixed(1)} KiB` : "none"],
    ["model", `${answer.metadata.provider} · ${answer.metadata.model}`],
    [
      "prompts",
      [answer.metadata.interpret_prompt_version, answer.metadata.synthesize_prompt_version]
        .filter(Boolean)
        .join(", "),
    ],
  ];
  return (
    <details className="rounded-[4px] border border-border bg-bg px-3 py-2 text-[12px]">
      <summary className="flex cursor-pointer items-center gap-1.5 text-text-2">
        <ListTree size={12} /> Retrieval plan
      </summary>
      <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
        {rows
          .filter(([, value]) => value !== undefined && value !== "")
          .map(([label, value]) => (
            <div key={label} className="contents">
              <dt className="text-text-3">{label}</dt>
              <dd className="font-mono text-[11px] break-words text-text-2">{value}</dd>
            </div>
          ))}
      </dl>
      {plan.reason && <p className="mt-2 text-[11.5px] text-text-3">“{plan.reason}”</p>}
      <p className="mt-2 text-[11px] text-text-3">
        The model only described this plan; MindGraph Core validated it and ran it with bounded
        graph operations. Nothing was written to the graph.
      </p>
    </details>
  );
}

function Answer({
  run,
  evidenceShown,
  onShowOnGraph,
  onShowRef,
  onClearEvidence,
  onOpenEntity,
}: {
  run: Extract<AskRun, { status: "completed" }>;
  evidenceShown: boolean;
  onShowOnGraph(): void;
  onShowRef(ref: string): void;
  onClearEvidence(): void;
  onOpenEntity(id: string): void;
}) {
  const { answer } = run;
  const status = STATUS[answer.status];
  const [copied, setCopied] = useState(false);
  const seconds = (answer.metadata.duration_ms / 1000).toFixed(1);
  const hasEvidence = answer.evidence.entities.length > 0;
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <Kicker>MindGraph</Kicker>
        <Badge tone={status.tone}>{status.label}</Badge>
        <span className="ml-auto font-mono text-[10px] text-text-3">
          {OPERATION[answer.retrieval.plan.operation] ?? answer.retrieval.plan.operation} ·{" "}
          {answer.retrieval.operations} ops · {seconds} s
        </span>
      </div>
      <AnswerText answer={answer} onShowRef={onShowRef} />
      {answer.answer_source !== "model" && (
        <p className="text-[11.5px] text-text-3">
          Written by MindGraph, not by the model: there was no graph evidence to phrase.
        </p>
      )}
      {answer.status === "partial" && (
        <p className="flex gap-2 rounded-[4px] border border-warn/40 bg-warn/10 px-2.5 py-2 text-[12px] text-warn">
          <TriangleAlert size={13} className="mt-0.5 shrink-0" />
          Bounded search: a retrieval limit was reached, so this answer may miss results. It does
          not cover the whole graph.
        </p>
      )}
      {answer.status === "insufficient_evidence" && (
        <p className="flex gap-2 rounded-[4px] border border-border-strong bg-raised px-2.5 py-2 text-[12px] text-text-2">
          <TriangleAlert size={13} className="mt-0.5 shrink-0 text-text-3" />
          The retrieved graph context does not support a definite answer.
        </p>
      )}
      {(answer.retrieval.limitations.length > 0 || answer.warnings.length > 0) && (
        <ul className="flex list-disc flex-col gap-1 pl-4 text-[11.5px] text-text-3">
          {[...answer.retrieval.limitations, ...answer.warnings].map((note, index) => (
            <li key={index}>{note}</li>
          ))}
        </ul>
      )}
      <Paths answer={answer} onShowRef={onShowRef} onOpenEntity={onOpenEntity} />
      <Evidence answer={answer} onShowRef={onShowRef} />
      <div className="flex flex-wrap items-center gap-2">
        {hasEvidence && (
          <Button variant="primary" icon={<ScanSearch size={14} />} onClick={onShowOnGraph}>
            Show on Graph
          </Button>
        )}
        {evidenceShown && (
          <Button icon={<Eraser size={14} />} onClick={onClearEvidence}>
            Clear Evidence Highlight
          </Button>
        )}
        <button
          type="button"
          aria-label="Copy answer"
          title="Copy answer"
          onClick={() => {
            void navigator.clipboard?.writeText(answer.answer).then(() => setCopied(true));
          }}
          className="ml-auto inline-flex items-center gap-1 text-[11px] text-text-3 hover:text-text"
        >
          <Copy size={13} />
          {copied && <span className="text-ok">copied</span>}
        </button>
      </div>
      <Plan answer={answer} />
    </div>
  );
}

/** An earlier question: compact, with its evidence one click away. */
function EarlierTurn({ turn, onShow }: { turn: AskTurn; onShow(): void }) {
  const status = STATUS[turn.answer.status];
  return (
    <article
      aria-label={`Earlier question: ${turn.question}`}
      className="flex flex-col gap-1.5 border-b border-border pb-3"
    >
      <Kicker>You · {turn.scope}</Kicker>
      <p className="text-[13px] font-semibold text-text-2">{turn.question}</p>
      <div className="flex items-center gap-2">
        <Kicker>MindGraph</Kicker>
        <Badge tone={status.tone}>{status.label}</Badge>
        {turn.answer.evidence.entities.length > 0 && (
          <button
            type="button"
            onClick={onShow}
            className="ml-auto inline-flex items-center gap-1 text-[11px] text-text-3 hover:text-text"
          >
            <ScanSearch size={12} /> Show on graph
          </button>
        )}
      </div>
      <p className="line-clamp-3 text-[12.5px] leading-[1.5] text-text-2">
        {turn.answer.answer.replace(/\s*\[[ERP]\d+(?:\s*,\s*[ERP]\d+)*\]/g, "")}
      </p>
    </article>
  );
}

function Unavailable({ availability }: { availability: AskAvailability }) {
  if (availability === "not_configured") {
    return (
      <section className="flex flex-col gap-2 rounded-[6px] border border-border bg-bg p-3">
        <p className="text-[14px] font-semibold">A model must be configured</p>
        <p className="text-[12.5px] text-text-2">
          Ask MindGraph needs an LLM provider. Set <code>MINDGRAPH_LLM_PROVIDER</code>,{" "}
          <code>MINDGRAPH_LLM_BASE_URL</code> and <code>MINDGRAPH_LLM_MODEL</code> (see{" "}
          <code>.env.example</code>) and restart. Exploring the graph keeps working without it.
        </p>
      </section>
    );
  }
  return (
    <section className="flex flex-col gap-2 rounded-[6px] border border-warn/40 bg-bg p-3">
      <p className="text-[14px] font-semibold">The AI service is unavailable</p>
      <p className="text-[12.5px] text-text-2">
        MindGraph cannot reach the AI service or the model provider right now. You can still ask;
        exploring the graph keeps working.
      </p>
    </section>
  );
}

export function AskPanel(props: AskPanelProps) {
  const { availability, selected, scope, question, run, thread } = props;
  const answering = run.status === "answering";
  const followUp = thread.length > 0 || run.status === "completed";
  const current = useRef<HTMLDivElement>(null);
  // The newest question and its answer come into view (below the thread).
  useEffect(() => {
    current.current?.scrollIntoView?.({ block: "start", behavior: "smooth" });
  }, [run.status, thread.length]);
  const disabled = availability === "not_configured" || answering;
  const trimmed = question.trim();
  const tooLong = question.length > ASK_LIMITS.questionMax;
  const canSubmit = !disabled && trimmed.length > 0 && !tooLong;

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (canSubmit) props.onSubmit();
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-border px-4 pt-4 pb-3">
        <MessageSquareText size={15} className="text-accent" />
        <h2 className="text-[15px] font-bold">Ask MindGraph</h2>
        <Badge>GraphRAG</Badge>
        {(followUp || run.status !== "idle") && (
          <button
            type="button"
            aria-label="New conversation"
            title="New conversation: forget the earlier questions"
            disabled={answering}
            onClick={props.onNewConversation}
            className="ml-auto text-text-3 hover:text-text disabled:opacity-40"
          >
            <History size={14} />
          </button>
        )}
        <button
          type="button"
          aria-label="Close Ask MindGraph"
          onClick={props.onClose}
          className={cn(
            "text-text-3 hover:text-text",
            !(followUp || run.status !== "idle") && "ml-auto",
          )}
        >
          <X size={14} />
        </button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 py-3">
        {(availability === "not_configured" || availability === "unavailable") && (
          <Unavailable availability={availability} />
        )}
        {run.status === "idle" ? (
          availability !== "not_configured" && (
            <div className="flex flex-col gap-2 text-[12.5px] text-text-2">
              <p>
                Ask a question about this graph. MindGraph retrieves evidence from the graph first;
                then the model phrases an answer that cites it, and you can show every reference on
                the graph.
              </p>
              <p className="text-[11.5px] text-text-3">
                Sent to the configured model: your question, the ontology, the selected
                entity&apos;s properties and the retrieved evidence (at most 150 entities); for a
                follow-up also your last {ASK_LIMITS.historyTurns} questions in this panel and the
                entities they cited. Nothing is written to the graph or stored.
              </p>
            </div>
          )
        ) : (
          <>
            {thread.map((turn, index) => (
              <EarlierTurn
                key={`${index}-${turn.question}`}
                turn={turn}
                onShow={() => props.onShowTurn(index)}
              />
            ))}
            <div ref={current} className="flex scroll-mt-3 flex-col gap-1">
              <Kicker>You · {run.scope}</Kicker>
              <p className="text-[14px] font-semibold">{run.question}</p>
            </div>
            {run.status === "answering" && (
              <p role="status" className="flex items-center gap-2 text-[12.5px] text-text-2">
                <LoaderCircle size={14} className="animate-spin" />
                Retrieving graph evidence and writing the answer…
              </p>
            )}
            {run.status === "error" && (
              <div className="flex flex-col gap-2">
                <ErrorNote>{run.message}</ErrorNote>
                <p className="text-[11.5px] text-text-3">The loaded graph is unchanged.</p>
                <Button size="sm" onClick={() => props.onSubmit(run.question)}>
                  Try again
                </Button>
              </div>
            )}
            {run.status === "completed" && (
              <Answer
                run={run}
                evidenceShown={props.evidenceShown}
                onShowOnGraph={props.onShowOnGraph}
                onShowRef={props.onShowRef}
                onClearEvidence={props.onClearEvidence}
                onOpenEntity={props.onOpenEntity}
              />
            )}
          </>
        )}
      </div>

      <form
        className="flex flex-col gap-2 border-t border-border px-4 py-3"
        onSubmit={(event) => {
          event.preventDefault();
          if (canSubmit) props.onSubmit();
        }}
      >
        <div role="radiogroup" aria-label="Question scope" className="flex flex-wrap gap-1.5">
          {selected && (
            <button
              type="button"
              role="radio"
              aria-checked={scope === "selected"}
              onClick={() => props.onScope("selected")}
              className={cn(
                "max-w-full truncate rounded-full border px-2.5 py-0.5 text-[11.5px]",
                scope === "selected"
                  ? "border-accent/60 bg-accent-deep text-accent"
                  : "border-border-strong text-text-3 hover:text-text",
              )}
            >
              Selected · {selected.label}
            </button>
          )}
          <button
            type="button"
            role="radio"
            aria-checked={scope === "graph"}
            onClick={() => props.onScope("graph")}
            className={cn(
              "rounded-full border px-2.5 py-0.5 text-[11.5px]",
              scope === "graph"
                ? "border-accent/60 bg-accent-deep text-accent"
                : "border-border-strong text-text-3 hover:text-text",
            )}
          >
            Whole graph (bounded)
          </button>
        </div>
        {/* Suggestions start a conversation; "New conversation" brings them back. */}
        {run.status === "idle" &&
          availability !== "not_configured" &&
          props.suggestions.length > 0 && (
            <div aria-label="Suggested questions" className="flex flex-wrap gap-1.5">
              {props.suggestions.map((suggestion) => (
                <button
                  key={suggestion.question}
                  type="button"
                  onClick={() =>
                    suggestion.submit
                      ? props.onSubmit(suggestion.question)
                      : props.onQuestion(suggestion.question)
                  }
                  className="rounded-[4px] border border-border-strong px-2 py-1 text-left text-[11.5px] text-text-2 hover:bg-raised hover:text-text"
                >
                  {suggestion.submit ? suggestion.question : `${suggestion.question}…`}
                </button>
              ))}
            </div>
          )}
        <div className="flex items-end gap-2 rounded-[4px] border border-border-strong bg-bg p-2 focus-within:border-accent/60">
          <textarea
            aria-label="Question"
            rows={2}
            placeholder={
              followUp
                ? "Ask a follow-up question…"
                : selected && scope === "selected"
                  ? `Ask about ${selected.label}…`
                  : "Ask anything about this graph…"
            }
            className="min-h-[36px] flex-1 resize-none bg-transparent text-[13px] outline-none placeholder:text-text-3"
            value={question}
            disabled={availability === "not_configured"}
            onChange={(event) => props.onQuestion(event.target.value)}
            onKeyDown={onKeyDown}
          />
          <button
            type="submit"
            aria-label="Ask"
            disabled={!canSubmit}
            className="inline-flex h-[30px] w-[30px] items-center justify-center rounded-[4px] bg-accent text-on-accent disabled:opacity-40"
          >
            {answering ? (
              <LoaderCircle size={14} className="animate-spin" />
            ) : (
              <ArrowUp size={14} />
            )}
          </button>
        </div>
        {followUp && (
          <p className="text-[11px] text-text-3">
            Follow-ups use your last {ASK_LIMITS.historyTurns} questions as context.
          </p>
        )}
        {tooLong && (
          <p className="text-[11px] text-danger">
            At most {ASK_LIMITS.questionMax} characters ({question.length} now).
          </p>
        )}
      </form>
    </div>
  );
}
