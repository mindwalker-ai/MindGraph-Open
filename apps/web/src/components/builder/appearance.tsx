/**
 * How entity types look in the builder: an icon and a colour per type, as
 * in the mockup (Customer blue, Account slate, Device amber, ...), from the
 * shared palette of `lib/graph/palette`.
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

import { type TypeKind, typeStyle } from "@/lib/graph/palette";
import { cn } from "@/lib/utils";

interface Look {
  icon: LucideIcon;
  /** A CSS colour (token variable). */
  color: string;
}

const ICONS: Record<TypeKind, LucideIcon> = {
  person: User,
  account: Landmark,
  transaction: ArrowLeftRight,
  merchant: Store,
  device: Smartphone,
  network: Globe,
  organization: Building2,
  document: FileText,
  other: Box,
};

export function lookOf(typeName: string): Look {
  const { kind, cssColor } = typeStyle(typeName);
  return { icon: ICONS[kind], color: cssColor };
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
