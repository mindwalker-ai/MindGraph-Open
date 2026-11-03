import type { DatasetProfile } from "@/lib/api/types";

/** The analysed fields of a dataset: inferred type, nullability, samples. */
export function ProfileTable({ profile }: { profile: DatasetProfile }) {
  return (
    <div className="overflow-x-auto">
      <p className="mb-3 text-sm text-muted">
        <span className="font-mono text-foreground">{profile.record_count.toLocaleString()}</span>{" "}
        records · <span className="font-mono text-foreground">{profile.fields.length}</span> fields
      </p>
      <table className="w-full text-left text-sm">
        <thead className="font-mono text-[11px] uppercase tracking-wider text-muted">
          <tr className="border-b border-border">
            <th className="py-1.5 pr-4 font-normal">Field</th>
            <th className="py-1.5 pr-4 font-normal">Type</th>
            <th className="py-1.5 pr-4 font-normal">Nulls</th>
            <th className="py-1.5 pr-4 font-normal">Non-null</th>
            <th className="py-1.5 font-normal">Samples</th>
          </tr>
        </thead>
        <tbody>
          {profile.fields.map((field) => (
            <tr key={field.name} className="border-b border-border/60 align-top">
              <td className="py-1.5 pr-4 font-mono">{field.name}</td>
              <td className="py-1.5 pr-4 font-mono text-accent">{field.inferred_type}</td>
              <td className="py-1.5 pr-4 text-muted">{field.nullable ? "nullable" : "never"}</td>
              <td className="py-1.5 pr-4 font-mono text-muted">{field.non_null_count}</td>
              <td className="py-1.5 font-mono text-xs text-muted">
                {field.sample_values.join(", ") || "—"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
