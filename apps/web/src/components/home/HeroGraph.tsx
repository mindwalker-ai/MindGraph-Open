/**
 * The decorative fraud graph of the welcome screen (mockup 01): two
 * customers linked through one shared device, with their accounts,
 * transactions, an IP address and a merchant. Static sample labels only.
 */

import {
  ArrowLeftRight,
  Globe,
  Landmark,
  type LucideIcon,
  Smartphone,
  Store,
  User,
} from "lucide-react";

type Kind = "device" | "customer" | "account" | "txn" | "ip" | "merchant";

const KINDS: Record<Kind, { icon: LucideIcon; color: string; size: number }> = {
  device: { icon: Smartphone, color: "var(--t-device)", size: 16 },
  customer: { icon: User, color: "var(--t-customer)", size: 16 },
  account: { icon: Landmark, color: "var(--t-account)", size: 14 },
  txn: { icon: ArrowLeftRight, color: "var(--t-txn)", size: 11 },
  ip: { icon: Globe, color: "var(--t-ip)", size: 15 },
  merchant: { icon: Store, color: "var(--t-merchant)", size: 18 },
};

interface HeroNode {
  id: string;
  x: number;
  y: number;
  kind: Kind;
  label?: string;
  faded?: boolean;
  shared?: boolean;
}

// Positions of the mockup's node boxes (120px wide, a 34px disc on top).
const NODES: HeroNode[] = [
  { id: "d921", x: 318, y: 112, kind: "device", label: "DEVICE-921", shared: true },
  { id: "d109", x: 24, y: 152, kind: "device", faded: true },
  { id: "d447", x: 0, y: 302, kind: "device", faded: true },
  { id: "d388", x: 620, y: 152, kind: "device", faded: true },
  { id: "cA", x: 166, y: 252, kind: "customer", label: "CUST-18291" },
  { id: "cB", x: 470, y: 252, kind: "customer", label: "CUST-20417" },
  { id: "cC", x: 644, y: 342, kind: "customer", faded: true },
  { id: "cD", x: -36, y: 452, kind: "customer", faded: true },
  { id: "a1029", x: 90, y: 414, kind: "account", label: "ACC-1029" },
  { id: "a3838", x: 224, y: 436, kind: "account", label: "ACC-3838" },
  { id: "a5512", x: 412, y: 436, kind: "account", label: "ACC-5512" },
  { id: "a6604", x: 546, y: 414, kind: "account", label: "ACC-6604" },
  { id: "a8812", x: 664, y: 504, kind: "account", faded: true },
  { id: "a2290", x: -26, y: 584, kind: "account", faded: true },
  { id: "ip", x: 318, y: 503, kind: "ip", label: "103.28.14.7", shared: true },
  { id: "t1", x: 54, y: 578, kind: "txn" },
  { id: "t2", x: 162, y: 600, kind: "txn" },
  { id: "t3", x: 254, y: 628, kind: "txn" },
  { id: "t4", x: 382, y: 628, kind: "txn" },
  { id: "t5", x: 474, y: 600, kind: "txn" },
  { id: "t6", x: 580, y: 578, kind: "txn" },
  { id: "t7", x: 674, y: 648, kind: "txn", faded: true },
  { id: "t8", x: -6, y: 708, kind: "txn", faded: true },
  { id: "mX", x: 318, y: 723, kind: "merchant" },
  { id: "mY", x: 594, y: 749, kind: "merchant", faded: true },
  { id: "mZ", x: 104, y: 759, kind: "merchant", faded: true },
];

// [from, to, label, highlighted]
const EDGES: [string, string, string?, boolean?][] = [
  ["cA", "d921", "USES", true],
  ["cB", "d921", "USES", true],
  ["cA", "d109"],
  ["cA", "d447"],
  ["cB", "d388"],
  ["cD", "d109"],
  ["cC", "a8812"],
  ["cA", "a1029", "OWNS"],
  ["cA", "a3838"],
  ["cB", "a5512"],
  ["cB", "a6604", "OWNS"],
  ["cD", "a2290"],
  ["a1029", "t1", "SENT"],
  ["a1029", "t2"],
  ["a3838", "t3"],
  ["a5512", "t4"],
  ["a6604", "t5"],
  ["a6604", "t6"],
  ["a8812", "t7"],
  ["a2290", "t8"],
  ["t1", "ip"],
  ["t2", "ip"],
  ["t3", "ip", "FROM_IP"],
  ["t4", "ip"],
  ["t5", "ip"],
  ["t6", "ip"],
  ["t1", "mX"],
  ["t2", "mX"],
  ["t3", "mX", "TO"],
  ["t4", "mX"],
  ["t5", "mX"],
  ["t6", "mX"],
  ["t7", "mY"],
  ["t8", "mZ"],
  ["t7", "mX"],
  ["t8", "mX"],
];

const byId = new Map(NODES.map((n) => [n.id, n]));
const center = (n: HeroNode) => ({ x: n.x + 60, y: n.y + 17 });

export function HeroGraph() {
  return (
    <div aria-hidden className="relative h-[824px] w-[780px]">
      {NODES.filter((n) => n.shared).map((n) => (
        <span
          key={`halo-${n.id}`}
          className="absolute h-[72px] w-[72px] rounded-full bg-risk/10"
          style={{ left: n.x + 24, top: n.y - 19 }}
        />
      ))}
      {/* Starts 36px to the left, where the faded nodes overhang. */}
      <svg className="absolute top-0 left-[-36px]" width={828} height={824} viewBox="-36 0 828 824">
        {EDGES.map(([from, to, , highlighted]) => {
          const a = byId.get(from);
          const b = byId.get(to);
          if (!a || !b) return null;
          const p = center(a);
          const q = center(b);
          return (
            <line
              key={`${from}-${to}`}
              x1={p.x}
              y1={p.y}
              x2={q.x}
              y2={q.y}
              stroke={highlighted ? "var(--risk)" : "var(--border-strong)"}
              strokeWidth={highlighted ? 1.5 : 1}
              opacity={a.faded || b.faded ? 0.15 : 1}
            />
          );
        })}
      </svg>
      {EDGES.filter(([, , label]) => label).map(([from, to, label, highlighted]) => {
        const p = center(byId.get(from) as HeroNode);
        const q = center(byId.get(to) as HeroNode);
        return (
          <span
            key={`label-${from}-${to}`}
            className="absolute -translate-x-1/2 -translate-y-1/2 rounded-[2px] bg-bg px-1 py-px font-mono text-[9px] font-medium tracking-[0.4px]"
            style={{
              left: (p.x + q.x) / 2,
              top: (p.y + q.y) / 2,
              color: highlighted ? "var(--risk)" : "var(--text-3)",
            }}
          >
            {label}
          </span>
        );
      })}
      {NODES.map((n) => {
        const { icon: Icon, color, size } = KINDS[n.kind];
        const ring = n.shared ? "var(--risk)" : color;
        return (
          <div
            key={n.id}
            className="absolute flex w-[120px] flex-col items-center gap-[5px]"
            style={{ left: n.x, top: n.y, opacity: n.faded ? 0.16 : 1 }}
          >
            <span
              className="flex h-[34px] w-[34px] items-center justify-center rounded-full bg-panel"
              style={{ boxShadow: `inset 0 0 0 1.5px ${ring}`, color: ring }}
            >
              <Icon size={size} />
            </span>
            {n.label && (
              <span className="rounded-[2px] bg-bg px-1 font-mono text-[10px] font-medium text-text-2">
                {n.label}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}
