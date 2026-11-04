/**
 * Colours of entity types, shared by the Graph Builder (CSS variables) and
 * the Graph Explorer (WebGL needs concrete colours). Chosen from the type
 * name, as in the mockup (Customer blue, Account slate, Device amber, ...);
 * unknown names get a deterministic colour from a small palette.
 */

/** A family of entity types; decides the icon in the builder. */
export type TypeKind =
  | "person"
  | "account"
  | "transaction"
  | "merchant"
  | "device"
  | "network"
  | "organization"
  | "document"
  | "other";

type Tone = "customer" | "account" | "txn" | "merchant" | "device" | "ip" | "risk";

/** The values of the `--t-*` and `--risk` tokens of `globals.css`. */
const HEX: Record<Tone, string> = {
  customer: "#5b9bff",
  account: "#93a4bc",
  txn: "#6e7887",
  merchant: "#a78bfa",
  device: "#e8b44c",
  ip: "#4fc3b8",
  risk: "#ff7a45",
};

const RULES: [RegExp, TypeKind, Tone][] = [
  [/customer|client|user|person|people|employee|member|owner/i, "person", "customer"],
  [/account|bank|wallet|card/i, "account", "account"],
  [/transaction|txn|payment|transfer|order|invoice/i, "transaction", "txn"],
  [/merchant|store|shop|vendor|seller/i, "merchant", "merchant"],
  [/device|phone|mobile|laptop|endpoint/i, "device", "device"],
  [/^ip|address|domain|network|host|url/i, "network", "ip"],
  [/company|organi[sz]ation|business|employer/i, "organization", "merchant"],
  [/document|file|report|note|article/i, "document", "account"],
];

const PALETTE: Tone[] = ["customer", "merchant", "device", "ip", "account", "risk"];

export interface TypeStyle {
  kind: TypeKind;
  /** A CSS colour (token variable) for the DOM. */
  cssColor: string;
  /** The same colour as a hex string, for the WebGL canvas. */
  hex: string;
}

function style(kind: TypeKind, tone: Tone): TypeStyle {
  return {
    kind,
    cssColor: tone === "risk" ? "var(--risk)" : `var(--t-${tone})`,
    hex: HEX[tone],
  };
}

/** How an entity type looks; the same name always gets the same style. */
export function typeStyle(typeName: string): TypeStyle {
  for (const [pattern, kind, tone] of RULES) {
    if (pattern.test(typeName)) return style(kind, tone);
  }
  let hash = 0;
  for (const char of typeName) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return style("other", PALETTE[hash % PALETTE.length]);
}
