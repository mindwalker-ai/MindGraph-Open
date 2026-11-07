"use client";

import { Check, Copy } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/primitives";

/** Copies `text` to the clipboard and says so for a moment. */
export function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(timer);
  }, [copied]);
  return (
    <Button
      size="sm"
      aria-label={label}
      icon={copied ? <Check size={12} /> : <Copy size={12} />}
      onClick={() => void navigator.clipboard?.writeText(text).then(() => setCopied(true))}
    >
      {copied ? "Copied" : "Copy"}
    </Button>
  );
}
