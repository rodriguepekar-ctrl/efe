import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "EFE Portal Admin",
  description: "EFE tenant administration — DPO + admin UI",
  robots: { index: false, follow: false },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className="h-full" suppressHydrationWarning>
      <body className="h-full bg-background font-sans antialiased">{children}</body>
    </html>
  );
}
