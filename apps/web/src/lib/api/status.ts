"use client";

import { useEffect, useState } from "react";

/** What `/api/ready` reports about MindGraph Core. */
export type CoreStatus = "checking" | "ready" | "not_ready" | "unreachable";

const POLL_MS = 30_000;

/** Core's readiness, checked on mount and every 30 seconds. */
export function useCoreStatus(): CoreStatus {
  const [status, setStatus] = useState<CoreStatus>("checking");
  useEffect(() => {
    let cancelled = false;
    const check = () =>
      fetch("/api/ready", { cache: "no-store" })
        .then((response) => response.json() as Promise<{ status: CoreStatus }>)
        .then(
          (body) => !cancelled && setStatus(body.status),
          () => !cancelled && setStatus("unreachable"),
        );
    void check();
    const timer = window.setInterval(() => void check(), POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);
  return status;
}

export const STATUS_LABEL: Record<CoreStatus, string> = {
  checking: "Checking MindGraph Core…",
  ready: "MindGraph Core is ready",
  not_ready: "MindGraph Core is up, but PostgreSQL or HugeGraph is not reachable",
  unreachable: "MindGraph Core cannot be reached",
};

export const STATUS_DOT: Record<CoreStatus, string> = {
  checking: "bg-text-3",
  ready: "bg-ok",
  not_ready: "bg-warn",
  unreachable: "bg-danger",
};
