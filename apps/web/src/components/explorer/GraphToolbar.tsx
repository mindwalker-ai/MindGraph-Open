"use client";

/**
 * The floating canvas toolbar (mockup 06). Every control acts; actions that
 * need a selection are disabled, with the reason in their tooltip, until
 * there is one.
 */

import {
  CircleDashed,
  Crosshair,
  Eraser,
  Expand,
  LoaderCircle,
  type LucideIcon,
  Maximize,
  Route,
  Waypoints,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { type RefObject, useCallback, useSyncExternalStore } from "react";

import type { CanvasHandle } from "@/components/explorer/canvas";
import type { LayoutMode } from "@/lib/graph/layout";
import { cn } from "@/lib/utils";

function Tool({
  icon: Icon,
  label,
  onClick,
  disabled,
  pressed,
  title,
  busy,
}: {
  icon: LucideIcon;
  label: string;
  onClick(): void;
  disabled?: boolean;
  pressed?: boolean;
  title?: string;
  busy?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-pressed={pressed}
      title={title ?? label}
      className={cn(
        "inline-flex h-7 items-center gap-1.5 rounded-[4px] px-2 text-[12px] text-text-2 hover:bg-hover hover:text-text disabled:cursor-not-allowed disabled:text-text-3 disabled:hover:bg-transparent",
        pressed && "bg-hover text-text",
      )}
    >
      {busy ? <LoaderCircle size={14} className="animate-spin" /> : <Icon size={14} />}
      <span className={cn(label.length > 0 ? "" : "sr-only")}>{label}</span>
    </button>
  );
}

function Divider() {
  return <span aria-hidden className="mx-0.5 h-4 w-px bg-border" />;
}

/** The camera zoom, read from the canvas without re-rendering the explorer. */
function useZoom(controllerRef: RefObject<CanvasHandle | null>, ready: boolean): number {
  const subscribe = useCallback(
    (listener: () => void) =>
      ready && controllerRef.current ? controllerRef.current.subscribe(listener) : () => {},
    [controllerRef, ready],
  );
  return useSyncExternalStore(
    subscribe,
    () => controllerRef.current?.zoomPercent() ?? 100,
    () => 100,
  );
}

export interface GraphToolbarProps {
  controllerRef: RefObject<CanvasHandle | null>;
  ready: boolean;
  hasGraph: boolean;
  layoutMode: LayoutMode;
  selectedEntity: string | null;
  expanding: boolean;
  canReset: boolean;
  onLayout(mode: LayoutMode): void;
  onExpand(): void;
  onFindPath(): void;
  onResetHighlight(): void;
}

export function GraphToolbar(props: GraphToolbarProps) {
  const { controllerRef, hasGraph } = props;
  const zoom = useZoom(controllerRef, props.ready);
  const camera = (action: (handle: CanvasHandle) => void) => {
    const handle = controllerRef.current;
    if (handle) action(handle);
  };
  const noSelection = props.selectedEntity ? undefined : "Select an entity first";
  return (
    <div
      role="toolbar"
      aria-label="Graph tools"
      className="pointer-events-auto flex items-center gap-0.5 rounded-[6px] border border-border-strong bg-panel/95 p-1 shadow-lg shadow-black/30 backdrop-blur"
    >
      <Tool icon={ZoomIn} label="" title="Zoom in" onClick={() => camera((c) => c.zoomIn())} />
      <Tool icon={ZoomOut} label="" title="Zoom out" onClick={() => camera((c) => c.zoomOut())} />
      <span className="w-11 text-center font-mono text-[10.5px] text-text-3" aria-label="Zoom">
        {zoom}%
      </span>
      <Tool
        icon={Maximize}
        label=""
        title="Fit the loaded graph"
        disabled={!hasGraph}
        onClick={() => camera((c) => c.fit())}
      />
      <Tool
        icon={Crosshair}
        label=""
        title={props.selectedEntity ? "Center on the selected entity" : "Center the view"}
        disabled={!hasGraph}
        onClick={() => camera((c) => c.center(props.selectedEntity))}
      />
      <Divider />
      <Tool
        icon={Waypoints}
        label="Force"
        title="Force-directed layout"
        pressed={props.layoutMode === "force"}
        disabled={!hasGraph}
        onClick={() => props.onLayout("force")}
      />
      <Tool
        icon={CircleDashed}
        label="Circular"
        title="Circular layout, grouped by entity type"
        pressed={props.layoutMode === "circular"}
        disabled={!hasGraph}
        onClick={() => props.onLayout("circular")}
      />
      <Divider />
      <Tool
        icon={Expand}
        label="Expand"
        title={noSelection ?? "Expand the selected entity (E)"}
        disabled={!props.selectedEntity || props.expanding}
        busy={props.expanding}
        onClick={props.onExpand}
      />
      <Tool
        icon={Route}
        label="Find Path"
        title="Find the shortest path between two entities (P)"
        onClick={props.onFindPath}
      />
      <Tool
        icon={Eraser}
        label=""
        title="Reset highlight and selection (Esc)"
        disabled={!props.canReset}
        onClick={props.onResetHighlight}
      />
    </div>
  );
}
