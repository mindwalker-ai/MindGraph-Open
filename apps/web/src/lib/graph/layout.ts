/**
 * Layouts of the loaded graph. ForceAtlas2 runs synchronously with a
 * bounded number of iterations (never a background loop); the view budget
 * keeps it to a few hundred nodes. Layouts return target positions, so the
 * canvas can animate to them.
 */

import { MultiDirectedGraph } from "graphology";
import forceAtlas2 from "graphology-layout-forceatlas2";

import type { ExplorerGraph } from "@/lib/graph/transform";

export type LayoutMode = "force" | "circular";

export const LAYOUT_MODES: { mode: LayoutMode; label: string }[] = [
  { mode: "force", label: "Force" },
  { mode: "circular", label: "Circular" },
];

export type Positions = Record<string, { x: number; y: number }>;

/** Fewer iterations for larger graphs, so a layout stays well under a second. */
export function forceIterations(order: number): number {
  if (order <= 100) return 300;
  if (order <= 300) return 150;
  return 80;
}

function settings(order: number) {
  return {
    ...forceAtlas2.inferSettings(order),
    gravity: 1,
    scalingRatio: 10,
    barnesHutOptimize: order > 300,
  };
}

/** A plain copy for ForceAtlas2, which reads `x`, `y`, `size` and `fixed`. */
function workingCopy(graph: ExplorerGraph, fixed: (id: string) => boolean) {
  const work = new MultiDirectedGraph<{ x: number; y: number; size: number; fixed: boolean }>({
    allowSelfLoops: true,
  });
  graph.forEachNode((id, { x, y, size }) => work.addNode(id, { x, y, size, fixed: fixed(id) }));
  graph.forEachEdge((edge, _, source, target) => work.addDirectedEdgeWithKey(edge, source, target));
  return work;
}

/** Force-directed positions for every loaded node. */
export function forceLayout(graph: ExplorerGraph): Positions {
  if (graph.order === 0) return {};
  return forceAtlas2(
    workingCopy(graph, () => false),
    {
      iterations: forceIterations(graph.order),
      settings: settings(graph.order),
    },
  );
}

/** Nodes on a circle, grouped by entity type, then by label. */
export function circularLayout(graph: ExplorerGraph): Positions {
  const nodes = graph
    .mapNodes((id, { entityType, label }) => ({ id, entityType, label }))
    .sort(
      (a, b) =>
        a.entityType.localeCompare(b.entityType) ||
        a.label.localeCompare(b.label) ||
        a.id.localeCompare(b.id),
    );
  const radius = Math.max(40, nodes.length * 4);
  const positions: Positions = {};
  nodes.forEach(({ id }, index) => {
    const angle = (2 * Math.PI * index) / nodes.length - Math.PI / 2;
    positions[id] = { x: radius * Math.cos(angle), y: radius * Math.sin(angle) };
  });
  return positions;
}

export function layoutPositions(graph: ExplorerGraph, mode: LayoutMode): Positions {
  return mode === "circular" ? circularLayout(graph) : forceLayout(graph);
}

/**
 * Lets newly added nodes settle among the existing ones, which stay where
 * they are, so an expansion does not rearrange the investigation. When
 * everything is new, the whole graph is laid out.
 */
export function settleNewNodes(graph: ExplorerGraph, added: string[], iterations = 80): void {
  if (added.length === 0) return;
  const fresh = new Set(added);
  const positions =
    fresh.size >= graph.order
      ? forceLayout(graph)
      : forceAtlas2(
          workingCopy(graph, (id) => !fresh.has(id)),
          { iterations, settings: settings(graph.order) },
        );
  for (const id of fresh) {
    if (graph.hasNode(id) && positions[id]) graph.mergeNodeAttributes(id, positions[id]);
  }
}
