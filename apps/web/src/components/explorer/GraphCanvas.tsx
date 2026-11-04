"use client";

/**
 * The WebGL canvas of the Graph Explorer: Sigma.js rendering the loaded
 * Graphology graph. Imported dynamically by the explorer, so it never runs
 * during server rendering. One Sigma instance lives as long as the
 * component; graph changes are picked up from Graphology events (no
 * remount), investigation state through the node and edge reducers.
 * Sigma, its listeners and running animations are disposed on unmount.
 */

import { EdgeCurvedArrowProgram } from "@sigma/edge-curve";
import { useEffect, useRef, useState } from "react";
import Sigma from "sigma";
import { EdgeArrowProgram, type NodeHoverDrawingFunction } from "sigma/rendering";
import { animateNodes } from "sigma/utils";

import type { CanvasHandle, CanvasProps } from "@/components/explorer/canvas";
import {
  type DisplayInput,
  type Emphasis,
  edgeAppearance,
  emphasis,
  nodeAppearance,
} from "@/lib/graph/state";
import type { EdgeAttributes, NodeAttributes } from "@/lib/graph/transform";

/** Below this camera ratio every visible relationship label is drawn. */
const EDGE_LABEL_RATIO = 0.5;

/** The fitted view leaves room around the graph for labels and the toolbar. */
const FIT_RATIO = 1.2;

/** The selected (or hovered) node: a ring and a label on a dark plate. */
const drawHover: NodeHoverDrawingFunction<NodeAttributes, EdgeAttributes> = (
  context,
  data,
  settings,
) => {
  context.beginPath();
  context.arc(data.x, data.y, data.size + 3, 0, Math.PI * 2);
  context.strokeStyle = "#e6e8eb";
  context.lineWidth = 1.5;
  context.stroke();
  if (!data.label) return;
  const size = settings.labelSize;
  context.font = `${settings.labelWeight} ${size}px ${settings.labelFont}`;
  const width = context.measureText(data.label).width;
  const x = data.x + data.size + 6;
  context.fillStyle = "#161a20";
  context.strokeStyle = "#2c323c";
  context.lineWidth = 1;
  context.beginPath();
  context.roundRect(x - 5, data.y - size / 2 - 5, width + 10, size + 10, 4);
  context.fill();
  context.stroke();
  context.fillStyle = "#e6e8eb";
  context.fillText(data.label, x, data.y + size / 3);
};

export default function GraphCanvas(props: CanvasProps) {
  const { graph, state, controllerRef } = props;
  const container = useRef<HTMLDivElement>(null);
  const renderer = useRef<Sigma<NodeAttributes, EdgeAttributes> | null>(null);
  const display = useRef<DisplayInput>({
    state,
    hoveredEntityId: null,
    hoveredRelationshipId: null,
    showAllEdgeLabels: false,
  });
  const handlers = useRef(props);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    handlers.current = props;
  });

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    const mono = getComputedStyle(element).getPropertyValue("--font-plex-mono").trim();
    const font = mono ? `${mono}, monospace` : "monospace";

    // What stands out is recomputed lazily: when the graph or the display
    // input (state, hover, zoom) changed since the last computation.
    let focus: Emphasis | null = null;
    let focusFor: DisplayInput | null = null;
    let graphChanged = true;
    const focusNow = () => {
      if (graphChanged || focusFor !== display.current) {
        focus = emphasis(graph, display.current);
        focusFor = display.current;
        graphChanged = false;
      }
      return focus;
    };
    const invalidate = () => {
      graphChanged = true;
    };

    let sigma: Sigma<NodeAttributes, EdgeAttributes>;
    try {
      sigma = new Sigma<NodeAttributes, EdgeAttributes>(graph, element, {
        allowInvalidContainer: true,
        renderEdgeLabels: true,
        enableEdgeEvents: true,
        zIndex: true,
        defaultEdgeType: "straight",
        edgeProgramClasses: { straight: EdgeArrowProgram, curved: EdgeCurvedArrowProgram },
        labelFont: font,
        labelSize: 11,
        labelWeight: "500",
        labelColor: { color: "#9aa2ad" },
        edgeLabelFont: font,
        edgeLabelSize: 9,
        edgeLabelColor: { color: "#9aa2ad" },
        labelRenderedSizeThreshold: 5,
        stagePadding: 72,
        minCameraRatio: 0.03,
        maxCameraRatio: 8,
        defaultDrawNodeHover: drawHover,
        nodeReducer: (node, data) => ({
          ...data,
          ...nodeAppearance(node, data, display.current, focusNow()),
        }),
        edgeReducer: (edge, data) => ({
          ...data,
          ...edgeAppearance(graph, edge, data, display.current, focusNow()),
        }),
      });
    } catch {
      // Most likely no WebGL; say so instead of showing a blank canvas.
      queueMicrotask(() =>
        setFailure("This browser cannot draw the graph (WebGL is unavailable)."),
      );
      return;
    }
    renderer.current = sigma;
    const camera = sigma.getCamera();
    const listeners = new Set<() => void>();
    const animations = new Set<() => void>();
    const refresh = () => {
      invalidate();
      sigma.refresh({ skipIndexation: true });
    };
    const graphEvents = [
      "nodeAdded",
      "edgeAdded",
      "nodeDropped",
      "edgeDropped",
      "cleared",
    ] as const;
    for (const name of graphEvents) graph.on(name, invalidate);

    // Mouse interactions.
    let dragged: string | null = null;
    sigma.on("clickNode", ({ node }) => handlers.current.onSelectEntity(node));
    sigma.on("clickEdge", ({ edge }) => handlers.current.onSelectRelationship(edge));
    sigma.on("clickStage", () => handlers.current.onClearSelection());
    sigma.on("doubleClickNode", (event) => {
      event.preventSigmaDefault();
      handlers.current.onExpand(event.node);
    });
    sigma.on("rightClickNode", ({ node, event }) => {
      event.preventSigmaDefault();
      event.original.preventDefault();
      handlers.current.onContextMenu(node, { x: event.x, y: event.y });
    });
    sigma.on("enterNode", ({ node }) => {
      display.current = { ...display.current, hoveredEntityId: node };
      element.style.cursor = "pointer";
      refresh();
    });
    sigma.on("leaveNode", () => {
      display.current = { ...display.current, hoveredEntityId: null };
      element.style.cursor = "";
      refresh();
    });
    sigma.on("enterEdge", ({ edge }) => {
      display.current = { ...display.current, hoveredRelationshipId: edge };
      element.style.cursor = "pointer";
      refresh();
    });
    sigma.on("leaveEdge", () => {
      display.current = { ...display.current, hoveredRelationshipId: null };
      element.style.cursor = "";
      refresh();
    });
    // Dragging a node moves it (client-side only); the frame stays put.
    sigma.on("downNode", ({ node }) => {
      dragged = node;
      if (!sigma.getCustomBBox()) sigma.setCustomBBox(sigma.getBBox());
    });
    sigma.on("moveBody", ({ event }) => {
      if (!dragged || !graph.hasNode(dragged)) return;
      const position = sigma.viewportToGraph(event);
      graph.mergeNodeAttributes(dragged, { x: position.x, y: position.y });
      event.preventSigmaDefault();
      event.original.preventDefault();
      event.original.stopPropagation();
    });
    const drop = () => {
      dragged = null;
    };
    sigma.on("upNode", drop);
    sigma.on("upStage", drop);

    camera.on("updated", ({ ratio }) => {
      const showAll = ratio < EDGE_LABEL_RATIO;
      if (showAll !== display.current.showAllEdgeLabels) {
        display.current = { ...display.current, showAllEdgeLabels: showAll };
        refresh();
      }
      for (const listener of listeners) listener();
    });

    const resize = new ResizeObserver(() => sigma.resize());
    resize.observe(element);

    /** Re-frames the graph to what is loaded now, so later additions do not shift it. */
    const reframe = () => {
      sigma.setCustomBBox(null);
      sigma.refresh();
      if (graph.order > 0) sigma.setCustomBBox(sigma.getBBox());
    };

    const handle: CanvasHandle = {
      zoomIn: () => void camera.animatedZoom({ duration: 200 }),
      zoomOut: () => void camera.animatedUnzoom({ duration: 200 }),
      fit: () => {
        reframe();
        void camera.animate({ x: 0.5, y: 0.5, ratio: FIT_RATIO, angle: 0 }, { duration: 300 });
      },
      center: (entityId) => {
        sigma.refresh();
        const point = entityId ? sigma.getNodeDisplayData(entityId) : null;
        void camera.animate(point ? { x: point.x, y: point.y } : { x: 0.5, y: 0.5 }, {
          duration: 300,
        });
      },
      focus: (entityIds) => {
        sigma.refresh();
        const points = entityIds.flatMap((id) => sigma.getNodeDisplayData(id) ?? []);
        if (points.length === 0) return;
        const xs = points.map((p) => p.x);
        const ys = points.map((p) => p.y);
        const span = Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys));
        const ratio =
          points.length === 1
            ? Math.min(camera.ratio, 0.6)
            : Math.min(Math.max(span * 1.5, 0.15), 2);
        void camera.animate(
          {
            x: (Math.min(...xs) + Math.max(...xs)) / 2,
            y: (Math.min(...ys) + Math.max(...ys)) / 2,
            ratio,
          },
          { duration: 400 },
        );
      },
      applyLayout: (positions) => {
        for (const cancel of animations) cancel();
        animations.clear();
        sigma.setCustomBBox(null);
        const cancel = animateNodes(
          graph,
          positions,
          { duration: 450, easing: "quadraticInOut" },
          () => {
            animations.delete(cancel);
            handle.fit();
          },
        );
        animations.add(cancel);
      },
      zoomPercent: () => Math.round((100 * FIT_RATIO) / camera.ratio),
      subscribe: (listener) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
    };
    controllerRef.current = handle;
    handlers.current.onReady();

    return () => {
      controllerRef.current = null;
      for (const cancel of animations) cancel();
      for (const name of graphEvents) graph.off(name, invalidate);
      resize.disconnect();
      listeners.clear();
      sigma.kill();
      renderer.current = null;
    };
  }, [graph, controllerRef]);

  // Investigation state changes only restyle; nothing is re-indexed.
  useEffect(() => {
    display.current = { ...display.current, state };
    renderer.current?.refresh({ skipIndexation: true });
  }, [state]);

  return (
    <div className="absolute inset-0" onContextMenu={(event) => event.preventDefault()}>
      <div ref={container} className="absolute inset-0" data-testid="graph-canvas" />
      {failure && (
        <p className="absolute inset-0 flex items-center justify-center text-[13px] text-text-3">
          {failure}
        </p>
      )}
    </div>
  );
}
