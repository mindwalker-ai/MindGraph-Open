/**
 * Follows a background job through the job API until it finishes. Progress
 * is whatever the server reports; no phases are invented.
 */

import type { ApiClient } from "@/lib/api/client";
import { type Job, isTerminal } from "@/lib/api/types";

export interface WaitOptions {
  intervalMs?: number;
  /** Stop waiting (for example when the component unmounts). */
  isCancelled?: () => boolean;
  onUpdate?: (job: Job) => void;
}

export class JobCancelled extends Error {
  constructor() {
    super("stopped waiting for the job");
    this.name = "JobCancelled";
  }
}

export async function waitForJob(
  api: ApiClient,
  jobId: string,
  { intervalMs = 750, isCancelled = () => false, onUpdate }: WaitOptions = {},
): Promise<Job> {
  for (;;) {
    if (isCancelled()) throw new JobCancelled();
    const job = await api.getJob(jobId);
    onUpdate?.(job);
    if (isTerminal(job.status)) return job;
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
}
