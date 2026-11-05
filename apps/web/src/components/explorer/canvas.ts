/**
 * The contract between the Graph Explorer and its canvas. The real canvas
 * (Sigma.js, WebGL) is loaded in the browser only; tests pass a stand-in.
 */

import type { RefObject } from "react";

import type { Positions } from "@/lib/graph/layout";
import type { InvestigationState } from "@/lib/graph/state";
import type { ExplorerGraph } from "@/lib/graph/transform";

/** Camera and layout commands the explorer sends to the canvas. */
export interface CanvasHandle {
  zoomIn(): void;
  zoomOut(): void;
  /** Fits every loaded entity in the viewport. */
  fit(): void;
  /** Centres on an entity, or on the loaded graph, keeping the zoom. */
  center(entityId?: string | null): void;
  /** Fits the camera to some entities (a path, a neighbourhood). */
  focus(entityIds: string[]): void;
  /** Moves the nodes to new positions (animated), then fits. */
  applyLayout(positions: Positions): void;
  /** The zoom level, 100 at the fitted view. */
  zoomPercent(): number;
  /** Called whenever the camera moves; returns the unsubscribe function. */
  subscribe(listener: () => void): () => void;
}

export interface CanvasProps {
  /** The loaded graph; the same instance for the explorer's lifetime. */
  graph: ExplorerGraph;
  state: InvestigationState;
  controllerRef: RefObject<CanvasHandle | null>;
  /** Called once the canvas can take commands. */
  onReady(): void;
  onSelectEntity(entityId: string): void;
  onSelectRelationship(relationshipId: string): void;
  onClearSelection(): void;
  onExpand(entityId: string): void;
  /** Right click on an entity, at a position relative to the canvas. */
  onContextMenu(entityId: string, position: { x: number; y: number }): void;
}
