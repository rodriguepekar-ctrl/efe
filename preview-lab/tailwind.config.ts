import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // Palette EFE : sombre + accents par axe (latence/coût/qualité).
        efe: {
          bg: "#0b0e14",
          panel: "#141a24",
          border: "#222b3a",
          muted: "#7b8794",
          text: "#e6edf3",
          accent: "#5ed3a0", // savings / vert
          cost: "#f2c66d", // coût / ambre
          latency: "#6db3f2", // latence / bleu
          danger: "#f2796d", // erreurs / rouge
          draft: "#9aa5b1", // tokens spéculatifs (fantôme)
        },
      },
      fontFamily: {
        mono: ["ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
      keyframes: {
        pulseNode: {
          "0%, 100%": { opacity: "1" },
          "50%": { opacity: "0.45" },
        },
      },
      animation: {
        pulseNode: "pulseNode 1s ease-in-out infinite",
      },
    },
  },
  plugins: [],
};

export default config;
