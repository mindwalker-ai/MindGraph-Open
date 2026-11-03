import { describe, expect, it } from "vitest";

import { cn } from "./utils";

describe("cn", () => {
  it("joins truthy class names and skips falsy ones", () => {
    const isActive = false;
    expect(cn("px-2", isActive && "bg-blue-500", undefined, "text-sm")).toBe("px-2 text-sm");
  });

  it("lets later Tailwind utilities override conflicting earlier ones", () => {
    expect(cn("px-2 py-1", "px-4")).toBe("py-1 px-4");
  });
});
