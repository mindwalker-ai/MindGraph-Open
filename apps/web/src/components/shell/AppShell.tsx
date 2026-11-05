"use client";

/**
 * The application shell of the mockup: a 220px sidebar (home, create and
 * upload pages) or a 52px icon rail (the Graph Builder), both listing the
 * same destinations. Destinations that later milestones build are shown,
 * disabled, with the milestone in their tooltip.
 */

import {
  BrainCircuit,
  Box,
  ChevronsUpDown,
  CodeXml,
  Database,
  type LucideIcon,
  MessageSquareText,
  Network,
  Package,
  Plug,
  Search,
  Settings,
  Spline,
  SquareTerminal,
  Waypoints,
} from "lucide-react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { type ReactNode, useSyncExternalStore } from "react";

import { Mark, comingIn } from "@/components/ui/primitives";
import { cn } from "@/lib/utils";

export const APP_VERSION = "v0.1.0";

interface NavItem {
  label: string;
  icon: LucideIcon;
  /** Where it leads, given the current graph; `null` when not available. */
  href: (graphId?: string) => string | null;
  milestone?: string;
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
      { label: "Search", icon: Search, href: () => null },
      {
        label: "Ask",
        icon: MessageSquareText,
        href: (graphId) => (graphId ? `/graphs/${graphId}/explore?ask=1` : null),
      },
      { label: "Queries", icon: SquareTerminal, href: () => null },
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
      { label: "Entities", icon: Box, href: () => null },
      { label: "Relationships", icon: Spline, href: () => null },
      {
        label: "Ontology",
        icon: Network,
        href: (graphId) => (graphId ? `/graphs/${graphId}/builder` : null),
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
    items: [
      { label: "API", icon: CodeXml, href: () => null },
      { label: "SDK", icon: Package, href: () => null },
    ],
  },
];

const SETTINGS: NavItem = { label: "Settings", icon: Settings, href: () => null };

/** The browser's host (`localhost:3000`), empty during server rendering. */
function useHost(): string {
  return useSyncExternalStore(
    () => () => {},
    () => window.location.host,
    () => "",
  );
}

function disabledTitle(item: NavItem, graphId?: string): string {
  if (item.milestone) return comingIn(item.milestone);
  if (!graphId && ["Graph", "Ask", "GraphRAG", "MCP", "Sources", "Ontology"].includes(item.label)) {
    return "Open a graph first";
  }
  return comingIn();
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
        title={disabledTitle(item, graphId)}
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
      <span
        aria-disabled
        title={comingIn()}
        className="flex w-full cursor-not-allowed items-center gap-2 rounded-[4px] border border-border px-2 py-[7px]"
      >
        <Search size={14} className="text-text-3" />
        <span className="flex-1 text-[13px] text-text-3">Search or run…</span>
        <span className="font-mono text-[10px] font-medium text-text-3">⌘K</span>
      </span>
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
      <SidebarItem item={SETTINGS} graphId={graphId} />
      <div className="flex items-center gap-2 border-t border-border px-2 pt-[10px]">
        <span className="h-1.5 w-1.5 rounded-full bg-ok" />
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
        title={`${item.label} · ${disabledTitle(item, graphId)}`}
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
      <RailItem item={SETTINGS} graphId={graphId} />
      <span className="mt-1 h-1.5 w-1.5 rounded-full bg-ok" title="Core is running" />
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
