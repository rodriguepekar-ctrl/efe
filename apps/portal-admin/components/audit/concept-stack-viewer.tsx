interface ConceptStackEntry {
  swarm_confidence: number;
  converged: boolean;
  top_anchors: { anchor_id: string; text: string; label: string; score: number }[];
  llm_reasoning: string;
  provider_used: string;
  template_id: string;
  tokens_used: number;
  cost_usd: number;
  duration_ms: number;
}

interface ConceptStack {
  entries: ConceptStackEntry[];
}

export function ConceptStackViewer({ stack }: { stack: ConceptStack }) {
  if (!stack || !stack.entries || stack.entries.length === 0) {
    return (
      <p className="rounded-md border bg-muted/30 p-4 text-sm text-muted-foreground">
        No concept stack recorded for this row.
      </p>
    );
  }

  return (
    <ol className="space-y-3">
      {stack.entries.map((entry, idx) => (
        // Index is acceptable here : the stack is append-only, never re-ordered.
        // biome-ignore lint/suspicious/noArrayIndexKey: stack.entries is append-only
        <li key={`turn-${idx}-${entry.provider_used}`}>
          <details
            className="rounded-md border p-4"
            open={idx === stack.entries.length - 1}
            data-testid={`turn-${idx + 1}`}
          >
            <summary className="cursor-pointer font-semibold">
              Turn {idx + 1} — confidence {entry.swarm_confidence.toFixed(3)}
              {entry.converged && (
                <span className="ml-2 rounded bg-green-100 px-2 py-0.5 text-xs text-green-900 dark:bg-green-950 dark:text-green-200">
                  ✓ converged
                </span>
              )}
            </summary>
            <div className="mt-3 space-y-3">
              <div>
                <h4 className="text-xs font-medium uppercase text-muted-foreground">
                  Top anchors ({entry.top_anchors.length})
                </h4>
                <ul className="mt-1 space-y-1 text-sm">
                  {entry.top_anchors.slice(0, 10).map((a) => (
                    <li key={a.anchor_id}>
                      <span className="font-mono text-xs text-muted-foreground">[{a.label}]</span>{" "}
                      {a.text}
                      <span className="ml-2 text-xs text-muted-foreground">
                        score {a.score.toFixed(3)}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
              <div>
                <h4 className="text-xs font-medium uppercase text-muted-foreground">
                  Provider / template
                </h4>
                <p className="mt-1 text-sm">
                  {entry.provider_used} / <code className="font-mono">{entry.template_id}</code>
                </p>
              </div>
              <div>
                <h4 className="text-xs font-medium uppercase text-muted-foreground">
                  LLM reasoning
                </h4>
                <pre className="mt-1 max-h-64 overflow-auto rounded bg-muted/50 p-3 text-xs">
                  {entry.llm_reasoning}
                </pre>
              </div>
              <p className="text-xs text-muted-foreground">
                {entry.tokens_used} tokens · ${entry.cost_usd.toFixed(5)} · {entry.duration_ms}ms
              </p>
            </div>
          </details>
        </li>
      ))}
    </ol>
  );
}
