"use client";

/**
 * Entity resolution in the Graph Explorer's details panel (M9, mockup 07's
 * "Potential duplicate" card).
 *
 * - A resolved entity shows its logical identity: the canonical record and
 *   how many original records are linked, each openable on its own.
 * - Pending candidates involving the entity show as potential duplicates
 *   with their ranking score (never a percentage) and what matched.
 *
 * The canvas is never changed: linked records stay separate nodes with
 * their own relationships.
 */

import { CopyCheck, ExternalLink, Fingerprint } from "lucide-react";
import Link from "next/link";
import { useEffect, useState } from "react";

import { Kicker } from "@/components/ui/primitives";
import type { ApiClient } from "@/lib/api/client";
import type { Entity, EntityIdentity, ResolutionCandidate } from "@/lib/api/types";
import { entityLabel, shortId } from "@/lib/graph/transform";
import { formatScore, matchedSummary } from "@/lib/resolution/rules";

interface Loaded {
  entityId: string;
  identity: EntityIdentity;
  pending: ResolutionCandidate[];
  pendingTotal: number;
  labels: Record<string, Entity>;
}

export function IdentityCard({
  api,
  graphId,
  entityId,
  onOpenEntity,
}: {
  api: ApiClient;
  graphId: string;
  entityId: string;
  onOpenEntity(entityId: string): void;
}) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [version, setVersion] = useState(0);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      api.getEntityIdentity(graphId, entityId),
      api.listResolutionCandidates(graphId, { status: "pending", entity_id: entityId, limit: 3 }),
    ]).then(
      async ([identity, page]) => {
        const others = new Set<string>();
        if (identity.resolved) identity.member_entity_ids.forEach((id) => others.add(id));
        for (const candidate of page.data) {
          others.add(candidate.entity_a_id);
          others.add(candidate.entity_b_id);
        }
        others.delete(entityId);
        const labels: Record<string, Entity> = {};
        if (others.size > 0) {
          try {
            const subgraph = await api.getSubgraph(graphId, [...others]);
            for (const entity of subgraph.entities) labels[entity.id] = entity;
          } catch {
            // Short ids stand in for labels.
          }
        }
        if (cancelled) return;
        setLoaded({
          entityId,
          identity,
          pending: page.data,
          pendingTotal: page.counts.pending,
          labels,
        });
      },
      () => {
        // Entity resolution is optional context here; the panel stays usable.
        if (!cancelled) setLoaded(null);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [api, graphId, entityId, version]);

  const current = loaded?.entityId === entityId ? loaded : null;
  if (!current || (!current.identity.resolved && current.pending.length === 0)) return null;
  const { identity, pending, labels } = current;
  const name = (id: string) => (labels[id] ? entityLabel(labels[id]) : shortId(id));
  const resolutionHref = `/graphs/${graphId}/resolution`;

  const keepSeparate = async (candidate: ResolutionCandidate) => {
    try {
      await api.decideResolutionCandidate(graphId, candidate.id, {
        decision: "different_entities",
      });
    } finally {
      setVersion((n) => n + 1);
    }
  };

  return (
    <section
      aria-label="Entity resolution"
      className="flex flex-col gap-2 border-b border-border px-4 py-3"
    >
      {identity.resolved && (
        <div className="flex flex-col gap-2 rounded-[4px] border border-border-strong bg-raised p-2.5">
          <div className="flex items-center gap-2">
            <Fingerprint size={13} className="shrink-0 text-ok" />
            <span className="flex-1 text-[12.5px] font-semibold text-text">Resolved identity</span>
            <span className="font-mono text-[10.5px] text-text-3">logical only</span>
          </div>
          <p className="text-[12px] text-text-2">
            Canonical:{" "}
            <span className="text-text">
              {identity.is_canonical ? "this record" : name(identity.canonical_entity_id)}
            </span>
            <br />
            {identity.member_entity_ids.length} entity records linked
          </p>
          <ul className="flex flex-col">
            {identity.member_entity_ids
              .filter((id) => id !== entityId)
              .map((id) => (
                <li key={id}>
                  <button
                    type="button"
                    onClick={() => onOpenEntity(id)}
                    className="flex w-full items-center gap-2 rounded-[4px] px-1.5 py-1 text-left text-[12px] hover:bg-hover"
                    title="Open this original record; it stays a separate node"
                  >
                    <span className="truncate text-text">{name(id)}</span>
                    {id === identity.canonical_entity_id && (
                      <span className="font-mono text-[10px] text-ok">canonical</span>
                    )}
                    <span className="ml-auto font-mono text-[10.5px] text-text-3">
                      {shortId(id)}
                    </span>
                  </button>
                </li>
              ))}
          </ul>
          <div className="flex flex-wrap gap-1.5">
            <Link
              href={`${resolutionHref}?entity=${entityId}`}
              className="inline-flex h-6 items-center rounded-[4px] border border-border px-2.5 text-[12px] font-semibold text-text hover:bg-hover"
            >
              View Identity Group
            </Link>
            <Link
              href={resolutionHref}
              className="inline-flex h-6 items-center gap-1 rounded-[4px] px-2 text-[12px] text-text-2 hover:text-text"
            >
              <ExternalLink size={11} /> Open Entity Resolution
            </Link>
          </div>
        </div>
      )}
      {pending.map((candidate) => {
        const other =
          candidate.entity_a_id === entityId ? candidate.entity_b_id : candidate.entity_a_id;
        // Linked through other decisions: nothing left to separate in v0.1.
        const linked = identity.resolved && identity.member_entity_ids.includes(other);
        return (
          <div
            key={candidate.id}
            className="flex flex-col gap-2 rounded-[4px] border border-border-strong bg-raised p-2.5"
          >
            <div className="flex items-center gap-2">
              <CopyCheck size={13} className="shrink-0 text-warn" />
              <span className="flex-1 text-[12.5px] font-semibold text-text">
                Potential duplicate
              </span>
              <span
                className="font-mono text-[11px] font-medium text-warn"
                title="Ranking score for review order, not a probability"
              >
                score {formatScore(candidate.score)}
              </span>
            </div>
            <p className="text-[12px] text-text-2">
              {name(other)} · {matchedSummary(candidate.signals)}
            </p>
            {linked && (
              <p className="text-[11.5px] text-text-3">
                Already linked in this identity through other decisions.
              </p>
            )}
            <div className="flex gap-1.5">
              <Link
                href={`${resolutionHref}?candidate=${candidate.id}&entity=${entityId}`}
                className="inline-flex h-6 items-center rounded-[4px] border border-border bg-hover px-2.5 text-[12px] font-semibold text-text hover:bg-raised"
              >
                Review
              </Link>
              {!linked && (
                <button
                  type="button"
                  onClick={() => void keepSeparate(candidate)}
                  className="inline-flex h-6 items-center rounded-[4px] border border-border px-2.5 text-[12px] font-semibold text-text hover:bg-hover"
                >
                  Keep Separate
                </button>
              )}
            </div>
          </div>
        );
      })}
      {current.pendingTotal > pending.length && (
        <Link
          href={`${resolutionHref}?entity=${entityId}`}
          className="text-[11.5px] text-text-3 hover:text-text"
        >
          {current.pendingTotal - pending.length} more potential duplicates
        </Link>
      )}
      <Kicker className="text-[9.5px]">Logical identity · the graph is not merged</Kicker>
    </section>
  );
}
