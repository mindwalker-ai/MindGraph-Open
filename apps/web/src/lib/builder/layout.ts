/**
 * Positions of the ontology preview: entity types in layers that follow
 * relationship direction (sources above targets), at most three per row,
 * rows centred. Deterministic, so the canvas does not jump between renders.
 */

export const NODE_WIDTH = 212;
export const NODE_HEIGHT = 58;
const GAP_X = 56;
const GAP_Y = 96;
const PADDING_X = 40;
const PADDING_TOP = 96;
const PADDING_BOTTOM = 72;
const PER_ROW = 3;

export interface Layout {
  positions: Map<string, { x: number; y: number }>;
  width: number;
  height: number;
}

/** Layers by longest path from the roots; cycles are cut where met. */
function ranks(nodes: string[], edges: { source: string; target: string }[]): Map<string, number> {
  const rank = new Map(nodes.map((n) => [n, 0]));
  const known = new Set(nodes);
  const useful = edges.filter(
    (e) => e.source !== e.target && known.has(e.source) && known.has(e.target),
  );
  // Bounded relaxation, so cycles cannot loop forever.
  for (let pass = 0; pass < nodes.length; pass += 1) {
    let changed = false;
    for (const { source, target } of useful) {
      const next = (rank.get(source) ?? 0) + 1;
      if (next > (rank.get(target) ?? 0) && next < nodes.length) {
        rank.set(target, next);
        changed = true;
      }
    }
    if (!changed) break;
  }
  return rank;
}

export function layoutGraph(nodes: string[], edges: { source: string; target: string }[]): Layout {
  const rank = ranks(nodes, edges);
  const layers = new Map<number, string[]>();
  for (const node of nodes) {
    const r = rank.get(node) ?? 0;
    layers.set(r, [...(layers.get(r) ?? []), node]);
  }
  const rows: string[][] = [];
  const column = new Map<string, number>();
  for (const r of [...layers.keys()].sort((a, b) => a - b)) {
    // Order each layer by the mean column of its sources (fewer crossings).
    const mean = (node: string) => {
      const from = edges.filter((e) => e.target === node && column.has(e.source));
      return from.length
        ? from.reduce((sum, e) => sum + (column.get(e.source) ?? 0), 0) / from.length
        : Number.MAX_SAFE_INTEGER;
    };
    const layer = [...(layers.get(r) ?? [])].sort((a, b) => mean(a) - mean(b));
    for (let i = 0; i < layer.length; i += PER_ROW) {
      const row = layer.slice(i, i + PER_ROW);
      row.forEach((node, index) => column.set(node, index));
      rows.push(row);
    }
  }
  const widest = Math.max(1, ...rows.map((row) => row.length));
  const rowWidth = (count: number) => count * NODE_WIDTH + (count - 1) * GAP_X;
  const width = rowWidth(widest) + 2 * PADDING_X;
  const positions = new Map<string, { x: number; y: number }>();
  rows.forEach((row, rowIndex) => {
    const offset = PADDING_X + (rowWidth(widest) - rowWidth(row.length)) / 2;
    row.forEach((node, index) => {
      positions.set(node, {
        x: offset + index * (NODE_WIDTH + GAP_X),
        y: PADDING_TOP + rowIndex * (NODE_HEIGHT + GAP_Y),
      });
    });
  });
  const height =
    PADDING_TOP + rows.length * NODE_HEIGHT + Math.max(0, rows.length - 1) * GAP_Y + PADDING_BOTTOM;
  return { positions, width, height };
}

/**
 * Where an edge leaves and enters its nodes: bottom to top when the target
 * is lower, side to side on the same row, top to bottom when it is higher.
 */
export function edgeEnds(
  from: { x: number; y: number },
  to: { x: number; y: number },
): { x1: number; y1: number; x2: number; y2: number } {
  const cx = (p: { x: number }) => p.x + NODE_WIDTH / 2;
  if (to.y > from.y) {
    return { x1: cx(from), y1: from.y + NODE_HEIGHT, x2: cx(to), y2: to.y };
  }
  if (to.y < from.y) {
    return { x1: cx(from), y1: from.y, x2: cx(to), y2: to.y + NODE_HEIGHT };
  }
  const leftToRight = to.x > from.x;
  const cy = from.y + NODE_HEIGHT / 2;
  return {
    x1: leftToRight ? from.x + NODE_WIDTH : from.x,
    y1: cy,
    x2: leftToRight ? to.x : to.x + NODE_WIDTH,
    y2: cy,
  };
}
