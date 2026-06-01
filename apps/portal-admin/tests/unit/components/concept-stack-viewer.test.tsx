import { ConceptStackViewer } from "@/components/audit/concept-stack-viewer";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

const baseEntry = {
  swarm_confidence: 0.62,
  converged: false,
  top_anchors: [
    { anchor_id: "a01", text: "facturation tardive", label: "billing", score: 0.9 },
    { anchor_id: "a02", text: "réclamation", label: "service", score: 0.4 },
  ],
  llm_reasoning: "Considered candidates billing and service ; tied scoring.",
  provider_used: "anthropic-sonnet",
  template_id: "mode3_refine_v1",
  tokens_used: 145,
  cost_usd: 0.0009,
  duration_ms: 432,
};

describe("ConceptStackViewer", () => {
  it("renders empty state with no entries", () => {
    render(<ConceptStackViewer stack={{ entries: [] }} />);
    expect(screen.getByText(/no concept stack/i)).toBeInTheDocument();
  });

  it("renders one turn", () => {
    render(<ConceptStackViewer stack={{ entries: [baseEntry] }} />);
    expect(screen.getByText(/Turn 1/)).toBeInTheDocument();
    expect(screen.getByText(/anthropic-sonnet/)).toBeInTheDocument();
  });

  it("marks converged turn", () => {
    render(<ConceptStackViewer stack={{ entries: [{ ...baseEntry, converged: true }] }} />);
    expect(screen.getByText(/converged/)).toBeInTheDocument();
  });

  it("renders anchors", () => {
    render(<ConceptStackViewer stack={{ entries: [baseEntry] }} />);
    expect(screen.getByText(/facturation tardive/)).toBeInTheDocument();
    expect(screen.getByText(/réclamation/)).toBeInTheDocument();
  });

  it("renders multiple turns and opens the last by default", () => {
    const second = { ...baseEntry, swarm_confidence: 0.91, converged: true };
    render(<ConceptStackViewer stack={{ entries: [baseEntry, second] }} />);
    const turns = screen.getAllByTestId(/turn-/);
    expect(turns).toHaveLength(2);
    // Use attribute presence (happy-dom's HTMLDetailsElement.open
    // boolean reflection is unreliable across versions).
    expect(turns[1]?.hasAttribute("open")).toBe(true);
    expect(turns[0]?.hasAttribute("open")).toBe(false);
  });
});
