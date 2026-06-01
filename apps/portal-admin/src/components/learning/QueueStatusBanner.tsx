type Stats = {
  queue_enqueued: number;
  queue_labeling: number;
  queue_awaiting_review: number;
  queue_failed: number;
  oldest_open_age_seconds: number;
};

export function QueueStatusBanner({ stats }: { stats: Stats }) {
  const total = stats.queue_enqueued + stats.queue_labeling;
  return (
    <div className="mb-4 rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm">
      <div className="flex items-start gap-3">
        <span className="text-2xl">⚠</span>
        <div>
          <div className="font-semibold text-amber-900">
            Queue near capacity ({total.toLocaleString()} in flight)
          </div>
          <div className="text-amber-800 mt-1">
            New samples may be skipped due to back-pressure. Possible causes :
            LLM provider down, budget exhausted, reviewer fatigue.
          </div>
          <ul className="mt-2 list-disc list-inside text-xs text-amber-800">
            <li>Check LLM router status (M21)</li>
            <li>
              Check budget : <code>tenant_feature.m24_active_learning_budget_cents_per_day</code>
            </li>
            <li>Review pending : {stats.queue_awaiting_review} items waiting</li>
            {stats.queue_failed > 0 && (
              <li className="text-red-800 font-medium">
                {stats.queue_failed} failed items — see CLI : <code>efe-admin learning queue list --status FAILED</code>
              </li>
            )}
          </ul>
        </div>
      </div>
    </div>
  );
}
