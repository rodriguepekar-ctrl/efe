// Sprint Φ-D-LATACT D92 — web_vitals push to Prometheus.

import { onLCP, onINP, onCLS, onFCP, onTTFB } from 'web-vitals';

/**
 * Push Core Web Vitals to the EFE Prometheus instrumentation endpoint.
 * Triggered automatically on page load via portal/src/app/layout.tsx.
 */
export function initWebVitalsReporting(): void {
  const send = (metric: { name: string; value: number; id: string }) => {
    const body = JSON.stringify({
      name: metric.name,
      value: metric.value,
      id: metric.id,
      page: window.location.pathname,
      session: getSessionId(),
      ts: Date.now(),
    });
    // Use sendBeacon for reliability (= survives page unload).
    if (navigator.sendBeacon) {
      navigator.sendBeacon('/api/metrics/web-vitals', body);
    } else {
      fetch('/api/metrics/web-vitals', {
        method: 'POST',
        keepalive: true,
        body,
      }).catch(() => { /* best-effort */ });
    }
  };

  onLCP(send);   // Largest Contentful Paint
  onINP(send);   // Interaction to Next Paint
  onCLS(send);   // Cumulative Layout Shift
  onFCP(send);   // First Contentful Paint
  onTTFB(send);  // Time to First Byte
}

function getSessionId(): string {
  const key = 'efe_portal_session_id';
  let id = sessionStorage.getItem(key);
  if (!id) {
    id = crypto.randomUUID();
    sessionStorage.setItem(key, id);
  }
  return id;
}
