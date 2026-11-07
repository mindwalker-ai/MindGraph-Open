"use client";

/**
 * The application shell of the mockup: a 220px sidebar (home, create and
 * upload pages) or a 52px icon rail (the Graph Builder), both listing the
 * same destinations. Only destinations that exist in v0.1 are listed; the
 * graph-scoped ones are disabled until a graph is open.
 */

import {
  BrainCircuit,
  ChevronsUpDown,
  CodeXml,
  CopyCheck,
  Database,
  type LucideIcon,
  MessageSquareText,
  Network,
  Play,
  Plug,
  Waypoints,
} from "lucide-react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { type ReactNode, useSyncExternalStore } from "react";

import { Mark } from "@/components/ui/primitives";
import { STATUS_DOT, STATUS_LABEL, useCoreStatus } from "@/lib/api/status";
import { cn } from "@/lib/utils";

export const APP_VERSION = "v0.1.0-alpha.1";

/** The REST API reference, kept with the source. */
const API_DOCS = "https://github.com/mindwalker-ai/MindGraph-Open/blob/main/docs/api/README.md";

interface NavItem {
  label: string;
  icon: LucideIcon;
  /** Where it leads, given the current graph; `null` until a graph is open. */
  href: (graphId?: string) => string | null;
  /** Opens outside the app, in a new tab. */
  external?: boolean;
}

const GROUPS: { label: string; items: NavItem[] }[] = [
  {
    label: "WORKSPACE",
    items: [
      {
        label: "Graph",
        icon: Waypoints,
        href: (graphId) => (graphId ? `/graphs/${graphId}/explore` : null),
      },
      {
        label: "Ask",
        icon: MessageSquareText,
        href: (graphId) => (graphId ? `/graphs/${graphId}/explore?ask=1` : null),
      },
      { label: "Demos", icon: Play, href: () => "/demos" },
    ],
  },
  {
    label: "DATA",
    items: [
      {
        label: "Sources",
        icon: Database,
        href: (graphId) => (graphId ? `/graphs/${graphId}/upload` : null),
      },
      {
        label: "Ontology",
        icon: Network,
        href: (graphId) => (graphId ? `/graphs/${graphId}/builder` : null),
      },
      {
        label: "Resolution",
        icon: CopyCheck,
        href: (graphId) => (graphId ? `/graphs/${graphId}/resolution` : null),
      },
    ],
  },
  {
    label: "AI",
    items: [
      {
        label: "GraphRAG",
        icon: BrainCircuit,
        href: (graphId) => (graphId ? `/graphs/${graphId}/explore?ask=1` : null),
      },
      {
        label: "MCP",
        icon: Plug,
        href: (graphId) => (graphId ? `/graphs/${graphId}/integrations/mcp` : null),
      },
    ],
  },
  {
    label: "DEVELOPER",
    items: [{ label: "API", icon: CodeXml, href: () => API_DOCS, external: true }],
  },
];

/** The browser's host (`localhost:3000`), empty during server rendering. */
function useHost(): string {
  return useSyncExternalStore(
    () => () => {},
    () => window.location.host,
    () => "",
  );
}

/** Why a graph-scoped destination is unavailable. */
const DISABLED_TITLE = "Open a graph first";

/** Link props: in-app, or a new tab for documentation. */
function linkProps(item: NavItem) {
  return item.external ? { target: "_blank", rel: "noreferrer" } : {};
}

function SidebarItem({ item, graphId }: { item: NavItem; graphId?: string }) {
  const pathname = usePathname();
  const href = item.href(graphId);
  const Icon = item.icon;
  const content = (
    <>
      <Icon size={15} strokeWidth={1.75} className="shrink-0" />
      <span>{item.label}</span>
    </>
  );
  const base = "flex h-7 w-full items-center gap-[10px] rounded-[4px] px-2 text-[13px]";
  if (!href) {
    return (
      <span
        aria-disabled
        title={DISABLED_TITLE}
        className={cn(base, "cursor-not-allowed text-text-3")}
      >
        {content}
      </span>
    );
  }
  const active = pathname === href;
  return (
    <Link
      href={href}
      {...linkProps(item)}
      aria-current={active ? "page" : undefined}
      className={cn(
        base,
        active ? "bg-raised text-text" : "text-text-2 hover:bg-raised hover:text-text",
      )}
    >
      {content}
    </Link>
  );
}

export function Sidebar({ graphId, graphName }: { graphId?: string; graphName?: string }) {
  const host = useHost();
  const status = useCoreStatus();
  return (
    <aside className="sticky top-0 flex h-screen w-[220px] shrink-0 flex-col gap-1 overflow-y-auto border-r border-border bg-panel px-[10px] py-[14px]">
      <Link href="/" className="flex items-center gap-2 px-[6px] pt-[2px] pb-[10px]">
        <Mark size={22} />
        <span className="text-[16px] font-bold text-text">MindGraph</span>
        <span className="rounded-[3px] border border-border-strong px-[5px] py-px font-mono text-[9px] font-medium tracking-[0.6px] text-text-3">
          OPEN
        </span>
      </Link>
      <Link
        href="/"
        title="All graphs"
        className="flex w-full items-center gap-2 rounded-[4px] border border-border bg-raised px-2 py-[7px] hover:border-border-strong"
      >
        <span className="h-2 w-2 shrink-0 rounded-[2px] bg-risk" />
        <span className="flex-1 truncate text-[13px] text-text">{graphName ?? "All graphs"}</span>
        <ChevronsUpDown size={14} className="text-text-3" />
      </Link>
      <nav className="flex flex-col" aria-label="Main">
        {GROUPS.map((group) => (
          <div key={group.label} className="flex flex-col gap-px pt-[10px]">
            <span className="px-2 pb-1 font-mono text-[9.5px] font-medium tracking-[0.6px] text-text-3">
              {group.label}
            </span>
            {group.items.map((item) => (
              <SidebarItem key={item.label} item={item} graphId={graphId} />
            ))}
          </div>
        ))}
      </nav>
      <div className="flex-1" />
      <div
        className="flex items-center gap-2 border-t border-border px-2 pt-[10px]"
        title={STATUS_LABEL[status]}
      >
        <span className={cn("h-1.5 w-1.5 rounded-full", STATUS_DOT[status])} />
        <span className="sr-only">{STATUS_LABEL[status]}</span>
        <span className="flex-1 truncate font-mono text-[10px] font-medium text-text-3">
          {host}
        </span>
        <span className="font-mono text-[10px] font-medium text-text-3">{APP_VERSION}</span>
      </div>
    </aside>
  );
}

function RailItem({ item, graphId }: { item: NavItem; graphId?: string }) {
  const pathname = usePathname();
  const href = item.href(graphId);
  const Icon = item.icon;
  const base = "flex h-8 w-[34px] items-center justify-center rounded-[4px]";
  if (!href) {
    return (
      <span
        aria-disabled
        aria-label={item.label}
        title={`${item.label} · ${DISABLED_TITLE}`}
        className={cn(base, "cursor-not-allowed text-text-3")}
      >
        <Icon size={16} strokeWidth={1.75} />
      </span>
    );
  }
  const active = pathname === href;
  return (
    <Link
      href={href}
      {...linkProps(item)}
      aria-label={item.label}
      title={item.label}
      className={cn(
        base,
        active ? "bg-raised text-text" : "text-text-2 hover:bg-raised hover:text-text",
      )}
    >
      <Icon size={16} strokeWidth={1.75} />
    </Link>
  );
}

export function Rail({ graphId }: { graphId?: string }) {
  const status = useCoreStatus();
  return (
    <aside className="sticky top-0 flex h-screen w-[52px] shrink-0 flex-col items-center gap-1 border-r border-border bg-panel py-[14px]">
      <Link href="/" aria-label="All graphs" className="pb-[10px]">
        <Mark size={24} />
      </Link>
      {GROUPS.map((group, index) => (
        <div key={group.label} className="flex flex-col items-center gap-1">
          {index > 0 && <span className="my-1 h-px w-5 bg-border" />}
          {group.items.map((item) => (
            <RailItem key={item.label} item={item} graphId={graphId} />
          ))}
        </div>
      ))}
      <div className="flex-1" />
      <span
        role="status"
        aria-label={STATUS_LABEL[status]}
        title={STATUS_LABEL[status]}
        className={cn("mt-1 h-1.5 w-1.5 rounded-full", STATUS_DOT[status])}
      />
    </aside>
  );
}

/** A page with the sidebar. */
export function AppShell({
  graphId,
  graphName,
  children,
}: {
  graphId?: string;
  graphName?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-screen">
      <Sidebar graphId={graphId} graphName={graphName} />
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}
