// SPDX-License-Identifier: UNLICENSED
import type { Metadata } from "next";
import type { ReactNode } from "react";

import "./globals.css";

export const metadata: Metadata = {
  title: "EFE · Preview Lab",
  description:
    "Phase 4 — visualisation live du pré-mâchage EFE (missions A/B/Speculative, savings).",
};

const SCENES: { href: string; label: string; ready: boolean }[] = [
  // Trailing slash : cohérent avec `trailingSlash: true` (export statique servi
  // par ServeDir) ET avec le dev server Next.
  { href: "/", label: "Live Mission", ready: true },
  { href: "/savings/", label: "Savings Report", ready: true },
  { href: "/ab/", label: "A/B Lab", ready: true },
  { href: "/resilience/", label: "Resilience", ready: true },
];

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="fr">
      <body>
        <div className="mx-auto flex min-h-screen max-w-6xl flex-col">
          <header className="flex items-center justify-between border-b border-efe-border px-4 py-3">
            <div className="flex items-baseline gap-2">
              <span className="text-lg font-bold text-efe-accent">EFE</span>
              <span className="text-sm text-efe-muted">Preview Lab</span>
            </div>
            <nav className="flex gap-1">
              {SCENES.map((s) =>
                s.ready ? (
                  <a
                    key={s.href}
                    href={s.href}
                    className="rounded-lg px-3 py-1 text-sm text-efe-text hover:bg-efe-panel"
                  >
                    {s.label}
                  </a>
                ) : (
                  <span
                    key={s.href}
                    title="à venir (P4.S5/S6)"
                    className="cursor-not-allowed rounded-lg px-3 py-1 text-sm text-efe-muted/50"
                  >
                    {s.label}
                  </span>
                ),
              )}
            </nav>
          </header>
          <main className="flex-1 p-4">{children}</main>
          <footer className="border-t border-efe-border px-4 py-2 text-[11px] text-efe-muted">
            bridge : <span className="font-mono">efe-preview-server</span> · protocole SSE v1.0.0
          </footer>
        </div>
      </body>
    </html>
  );
}
