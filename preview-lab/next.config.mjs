/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // P4.S6 — export 100 % statique : `next build` produit `out/` (HTML + JS),
  // servi par le bridge `efe-preview-server` via ServeDir (single-binary démo).
  // L'app est entièrement client-rendered (toutes les pages "use client",
  // data fetchée au runtime), donc compatible export.
  output: "export",
  // `trailingSlash` ⇒ `out/savings/index.html` ; ServeDir
  // (append_index_html_on_directories) sert `/savings/`.
  trailingSlash: true,
  // Le typecheck est couvert par `pnpm run typecheck` (tsc strict) ; on n'exige
  // pas une config ESLint pour produire un build (outil dev).
  eslint: { ignoreDuringBuilds: true },
};

export default nextConfig;
