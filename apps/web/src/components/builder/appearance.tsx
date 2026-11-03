/**
 * How entity types look in the builder: an icon and a colour per type, as
 * in the mockup (Customer blue, Account slate, Device amber, ...). Chosen
 * from the type name; anything unknown cycles through the palette.
 */

import {
  ArrowLeftRight,
  Box,
  Building2,
  FileText,
  Globe,
  Landmark,
  type LucideIcon,
  Smartphone,
  Store,
  User,
} from "lucide-react";

import { cn } from "@/lib/utils";

interface Look {
  icon: LucideIcon;
  /** A CSS colour (token variable). */
  color: string;
}

const RULES: [RegExp, Look][] = [
  [
    /customer|client|user|person|people|employee|member|owner/i,
    { icon: User, color: "var(--t-customer)" },
  ],
  [/account|bank|wallet|card/i, { icon: Landmark, color: "var(--t-account)" }],
  [
    /transaction|txn|payment|transfer|order|invoice/i,
    { icon: ArrowLeftRight, color: "var(--t-txn)" },
  ],
  [/merchant|store|shop|vendor|seller/i, { icon: Store, color: "var(--t-merchant)" }],
  [/device|phone|mobile|laptop|endpoint/i, { icon: Smartphone, color: "var(--t-device)" }],
  [/^ip|address|domain|network|host|url/i, { icon: Globe, color: "var(--t-ip)" }],
  [/company|organi[sz]ation|business|employer/i, { icon: Building2, color: "var(--t-merchant)" }],
  [/document|file|report|note|article/i, { icon: FileText, color: "var(--t-account)" }],
];

const PALETTE = [
  "var(--t-customer)",
  "var(--t-merchant)",
  "var(--t-device)",
  "var(--t-ip)",
  "var(--t-account)",
  "var(--risk)",
];

export function lookOf(typeName: string): Look {
  for (const [pattern, look] of RULES) {
    if (pattern.test(typeName)) return look;
  }
  let hash = 0;
  for (const char of typeName) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return { icon: Box, color: PALETTE[hash % PALETTE.length] };
}

/** The round outlined icon of an entity type. */
export function TypeDisc({
  typeName,
  size = 22,
  className,
}: {
  typeName: string;
  size?: number;
  className?: string;
}) {
  const { icon: Icon, color } = lookOf(typeName);
  return (
    <span
      aria-hidden
      className={cn("inline-flex shrink-0 items-center justify-center rounded-full", className)}
      style={{ width: size, height: size, boxShadow: `inset 0 0 0 1.5px ${color}`, color }}
    >
      <Icon size={Math.round(size / 2)} strokeWidth={2} />
    </span>
  );
}
