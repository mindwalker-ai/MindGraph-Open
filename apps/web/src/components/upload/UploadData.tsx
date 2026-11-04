"use client";

/**
 * "Upload Data" (mockup 04): drop several CSV or JSON files; each one is
 * uploaded and analysed right away (rows, columns, detected headers). The
 * files' ids live in the URL (`?sources=`), so a reload keeps them.
 * "Analyze Data" opens the AI Graph Builder on all parsed files.
 */

import {
  ArrowLeft,
  ChevronRight,
  CircleCheck,
  CircleX,
  FileSpreadsheet,
  LoaderCircle,
  Plus,
  ScanSearch,
  Upload,
  X,
} from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { type DragEvent, useEffect, useMemo, useRef, useState } from "react";

import { AppShell } from "@/components/shell/AppShell";
import { Button, ErrorNote, Kicker } from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient } from "@/lib/api/client";
import type { DataSource, GraphProject } from "@/lib/api/types";
import { JobCancelled, waitForJob } from "@/lib/builder/jobs";
import { cn } from "@/lib/utils";

type Row =
  | { key: string; state: "uploading"; filename: string }
  | { key: string; state: "analysing" | "ready" | "failed"; filename: string; source: DataSource }
  | { key: string; state: "error"; filename: string; message: string };

function message(error: unknown): string {
  if (error instanceof ApiError) return error.message;
  return error instanceof Error ? error.message : String(error);
}

let rowCounter = 0;
const nextRowKey = () => `r-${(rowCounter += 1)}`;

const COLUMNS = [
  ["FILE", "flex-1"],
  ["ROWS", "w-[110px]"],
  ["COLUMNS", "w-[100px]"],
  ["DETECTED HEADERS", "hidden w-[400px] xl:block"],
  ["STATUS", "w-[110px]"],
  ["", "w-7"],
] as const;

export function UploadData({
  graphId,
  initialSources = [],
  api: injected,
  pollMs = 750,
}: {
  graphId: string;
  initialSources?: string[];
  api?: ApiClient;
  pollMs?: number;
}) {
  const api = useMemo(() => injected ?? createApiClient(), [injected]);
  const router = useRouter();
  const alive = useRef(true);
  const input = useRef<HTMLInputElement>(null);
  const [graph, setGraph] = useState<GraphProject | null>(null);
  // Files of the URL, restored once (the page remounts per graph).
  const [restored] = useState(() => initialSources.map((id) => ({ id, key: nextRowKey() })));
  const [rows, setRows] = useState<Row[]>(() =>
    restored.map(({ key }) => ({ key, state: "uploading", filename: "…" })),
  );
  const [previewKey, setPreviewKey] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const update = (key: string, next: Row) =>
    setRows((all) => all.map((row) => (row.key === key ? next : row)));

  async function analyse(key: string, source: DataSource) {
    update(key, { key, state: "analysing", filename: source.filename, source });
    try {
      const accepted = await api.analyze(graphId, source.id);
      await waitForJob(api, accepted.job_id, {
        intervalMs: pollMs,
        isCancelled: () => !alive.current,
      });
      const analysed = await api.getDataSource(graphId, source.id);
      update(key, {
        key,
        state: analysed.status === "ready" ? "ready" : "failed",
        filename: analysed.filename,
        source: analysed,
      });
    } catch (err) {
      if (!(err instanceof JobCancelled)) {
        update(key, { key, state: "error", filename: source.filename, message: message(err) });
      }
    }
  }

  // Load the graph and restore the files listed in the URL, once.
  useEffect(() => {
    let cancelled = false;
    api.getGraph(graphId).then(
      (g) => !cancelled && setGraph(g),
      (err: unknown) => !cancelled && setError(message(err)),
    );
    for (const { id, key } of restored) {
      api.getDataSource(graphId, id).then(
        (source) => {
          if (cancelled) return;
          if (source.status === "pending") {
            void analyse(key, source);
          } else {
            update(key, {
              key,
              state: source.status === "ready" ? "ready" : "failed",
              filename: source.filename,
              source,
            });
          }
        },
        (err: unknown) =>
          !cancelled && update(key, { key, state: "error", filename: id, message: message(err) }),
      );
    }
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- restore once per graph
  }, [api, graphId]);

  const ready = rows.flatMap((row) => (row.state === "ready" ? [row.source] : []));
  const busy = rows.some((row) => row.state === "uploading" || row.state === "analysing");
  const sourceIds = rows.flatMap((row) => ("source" in row ? [row.source.id] : [])).join(",");

  // Keep the URL in step so a reload restores the same files.
  useEffect(() => {
    const url = sourceIds
      ? `/graphs/${graphId}/upload?sources=${sourceIds}`
      : `/graphs/${graphId}/upload`;
    window.history.replaceState(window.history.state, "", url);
  }, [graphId, sourceIds]);

  async function add(files: FileList | File[]) {
    setError(null);
    for (const file of Array.from(files)) {
      const key = nextRowKey();
      setRows((all) => [...all, { key, state: "uploading", filename: file.name }]);
      setPreviewKey((current) => current ?? key);
      try {
        const source = await api.uploadDataSource(graphId, file);
        void analyse(key, source);
      } catch (err) {
        update(key, { key, state: "error", filename: file.name, message: message(err) });
      }
    }
  }

  function onDrop(event: DragEvent) {
    event.preventDefault();
    setDragging(false);
    if (event.dataTransfer.files.length) void add(event.dataTransfer.files);
  }

  const totals = ready.reduce(
    (sum, source) => ({
      rows: sum.rows + (source.profile?.record_count ?? 0),
      columns: sum.columns + (source.profile?.fields.length ?? 0),
    }),
    { rows: 0, columns: 0 },
  );
  const previewRow =
    rows.find((row) => row.key === previewKey && row.state === "ready") ??
    rows.find((row) => row.state === "ready");
  const preview = previewRow && "source" in previewRow ? previewRow.source : null;
  const problems = rows.flatMap((row) =>
    row.state === "error"
      ? [`${row.filename}: ${row.message}`]
      : row.state === "failed"
        ? [`${row.filename}: the file could not be parsed`]
        : [],
  );

  return (
    <AppShell graphId={graphId} graphName={graph?.name}>
      <main className="flex min-h-screen flex-col gap-6 px-14 py-10">
        <nav aria-label="Breadcrumb" className="flex items-center gap-2 text-[13px]">
          <Link href="/graphs/new" className="text-text-3 hover:text-text-2">
            Create Graph
          </Link>
          <ChevronRight size={12} className="text-text-3" />
          <span className="font-semibold">Upload Data</span>
        </nav>
        <header className="flex flex-col gap-1">
          <h1 className="text-[30px] font-bold">Upload Data</h1>
          <p className="text-[14px] text-text-2">
            Add the files you want to turn into a graph. Relationships across files are detected
            automatically.
          </p>
        </header>

        <div
          onDragOver={(event) => {
            event.preventDefault();
            setDragging(true);
          }}
          onDragLeave={() => setDragging(false)}
          onDrop={onDrop}
          className={cn(
            "flex h-[150px] flex-col items-center justify-center gap-[10px] rounded-[8px] border border-dashed bg-panel",
            dragging ? "border-accent" : "border-border-strong",
          )}
        >
          <span className="flex h-10 w-10 items-center justify-center rounded-[8px] border border-border-strong text-text-2">
            <Upload size={18} />
          </span>
          <span className="text-[16px] font-bold">Drop your files here</span>
          <span className="flex items-center gap-[10px]">
            <span className="font-mono text-[10.5px] font-medium tracking-[0.6px] text-text-3">
              CSV OR JSON
            </span>
            <span className="text-[13px] text-text-3">or</span>
            <Button size="sm" onClick={() => input.current?.click()}>
              Browse Files
            </Button>
          </span>
          <input
            ref={input}
            type="file"
            multiple
            accept=".csv,.json,text/csv,application/json"
            aria-label="Dataset files"
            className="hidden"
            onChange={(event) => {
              if (event.target.files?.length) void add(event.target.files);
              event.target.value = "";
            }}
          />
        </div>

        {rows.length > 0 && (
          <section
            aria-label="Files"
            className="flex flex-col overflow-hidden rounded-[8px] border border-border"
          >
            <div className="flex h-8 items-center gap-3 border-b border-border bg-panel px-4">
              {COLUMNS.map(([label, width]) => (
                <Kicker key={label || "remove"} className={cn("text-[9.5px]", width)}>
                  {label}
                </Kicker>
              ))}
            </div>
            {rows.map((row) => {
              const profile = "source" in row ? row.source.profile : null;
              const selected = preview !== null && "source" in row && row.source.id === preview.id;
              return (
                <div
                  key={row.key}
                  className={cn(
                    "flex h-12 items-center gap-3 border-b border-border px-4",
                    selected && "bg-raised",
                  )}
                >
                  <button
                    type="button"
                    onClick={() => setPreviewKey(row.key)}
                    className="flex min-w-0 flex-1 items-center gap-[10px] text-left"
                  >
                    <FileSpreadsheet size={16} className="shrink-0 text-text-2" />
                    <span className="truncate font-mono text-[12px] font-medium">
                      {row.filename}
                    </span>
                  </button>
                  <span className="w-[110px] text-[14px] font-semibold">
                    {profile ? profile.record_count.toLocaleString() : "—"}
                  </span>
                  <span className="w-[100px] text-[14px] font-semibold">
                    {profile ? profile.fields.length : "—"}
                  </span>
                  <span className="hidden w-[400px] truncate font-mono text-[11px] font-medium text-text-2 xl:block">
                    {profile?.fields.map((f) => f.name).join(", ") ?? ""}
                  </span>
                  <span className="flex w-[110px] items-center gap-1.5 text-[12.5px] text-text-2">
                    {row.state === "ready" && (
                      <>
                        <CircleCheck size={13} className="text-ok" /> Parsed
                      </>
                    )}
                    {(row.state === "uploading" || row.state === "analysing") && (
                      <>
                        <LoaderCircle size={13} className="animate-spin text-accent" />
                        {row.state === "uploading" ? "Uploading" : "Parsing"}
                      </>
                    )}
                    {(row.state === "failed" || row.state === "error") && (
                      <span className="flex items-center gap-1.5 text-danger">
                        <CircleX size={13} /> Failed
                      </span>
                    )}
                  </span>
                  <span className="flex w-7 justify-end">
                    <button
                      type="button"
                      aria-label={`Remove ${row.filename}`}
                      title="Leave this file out of the graph (the stored upload is kept)"
                      disabled={row.state === "uploading" || row.state === "analysing"}
                      onClick={() => setRows((all) => all.filter((r) => r.key !== row.key))}
                      className="text-text-3 hover:text-text disabled:opacity-40"
                    >
                      <X size={13} />
                    </button>
                  </span>
                </div>
              );
            })}
            <div className="flex h-9 items-center gap-3 bg-panel px-4">
              <span className="flex-1 font-mono text-[10.5px] font-medium text-text-2">
                {ready.length} file{ready.length === 1 ? "" : "s"} · {totals.rows.toLocaleString()}{" "}
                rows · {totals.columns} columns
              </span>
              <button
                type="button"
                onClick={() => input.current?.click()}
                className="flex items-center gap-1.5 text-[12.5px] text-text-2 hover:text-text"
              >
                <Plus size={12} /> Add more files
              </button>
            </div>
          </section>
        )}

        {error && <ErrorNote>{error}</ErrorNote>}
        {problems.length > 0 && <ErrorNote>{problems.join(" · ")}</ErrorNote>}

        {preview?.profile && (
          <section aria-label="Preview" className="flex flex-col gap-[10px]">
            <div className="flex items-center gap-2">
              <span className="font-mono text-[10px] font-medium tracking-[0.8px] text-text-3">
                PREVIEW
              </span>
              <span className="font-mono text-[10px] font-medium text-text-2">
                {preview.filename} · first {Math.min(4, preview.profile.sample_rows.length)} of{" "}
                {preview.profile.record_count.toLocaleString()} rows
              </span>
            </div>
            <div className="overflow-x-auto rounded-[6px] border border-border">
              <table className="w-full font-mono text-[11px] font-medium">
                <thead>
                  <tr className="h-[30px] border-b border-border bg-panel text-left text-[10px] text-text-3">
                    {preview.profile.fields.map((field) => (
                      <th key={field.name} className="px-[14px] font-medium whitespace-nowrap">
                        {field.name}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {preview.profile.sample_rows.slice(0, 4).map((row, index) => (
                    <tr key={index} className="h-[30px] border-b border-border last:border-b-0">
                      {preview.profile?.fields.map((field) => {
                        const value = row[field.name];
                        return (
                          <td key={field.name} className="px-[14px] whitespace-nowrap text-text-2">
                            {value === null || value === undefined
                              ? ""
                              : typeof value === "string"
                                ? value
                                : JSON.stringify(value)}
                          </td>
                        );
                      })}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        )}

        <div className="flex-1" />
        <footer className="flex items-center gap-[10px]">
          <span className="flex-1 text-[12.5px] text-text-3">
            Next: MindGraph will propose entities and relationships for you to review.
          </span>
          <Link
            href="/"
            className="inline-flex h-8 items-center gap-2 rounded-[4px] border border-border-strong bg-raised px-3 text-[13px] font-semibold hover:bg-hover"
          >
            <ArrowLeft size={14} /> Back
          </Link>
          <Button
            variant="primary"
            size="lg"
            disabled={ready.length === 0 || busy}
            icon={<ScanSearch size={14} />}
            onClick={() =>
              router.push(`/graphs/${graphId}/builder?sources=${ready.map((s) => s.id).join(",")}`)
            }
          >
            Analyze Data
          </Button>
        </footer>
      </main>
    </AppShell>
  );
}
