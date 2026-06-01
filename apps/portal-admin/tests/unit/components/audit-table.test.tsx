import { AuditTable } from "@/components/audit/audit-table";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

describe("AuditTable", () => {
  const baseRow = {
    audit_id: 42,
    ts: "2026-05-09T10:00:00Z",
    tenant_id: "acme",
    mode: "MODE_2",
    sentence_hash: "0123456789abcdef0123456789abcdef0123456789abcdef",
    label: "billing",
    confidence: 0.871,
    duration_ms: 87,
    converged: null,
  };

  it("shows empty state when no rows", () => {
    render(<AuditTable rows={[]} />);
    expect(screen.getByText(/no audit rows/i)).toBeInTheDocument();
  });

  it("renders the row with formatted fields", () => {
    render(<AuditTable rows={[baseRow]} />);
    expect(screen.getByText("#42")).toBeInTheDocument();
    expect(screen.getByText(/0123456789abcdef/)).toBeInTheDocument();
    expect(screen.getByText("billing")).toBeInTheDocument();
    expect(screen.getByText("0.871")).toBeInTheDocument();
  });

  it("highlights MODE_3", () => {
    render(<AuditTable rows={[{ ...baseRow, mode: "MODE_3", converged: true }]} />);
    expect(screen.getByText("MODE_3")).toBeInTheDocument();
    expect(screen.getByText("✓")).toBeInTheDocument();
  });
});
