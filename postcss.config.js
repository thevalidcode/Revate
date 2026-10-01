// Tailwind v4 is wired up through the `@tailwindcss/vite` plugin (see
// vite.config.ts). It must NOT also run as a PostCSS plugin here, otherwise
// the two pipelines fight over the same `@import "tailwindcss"` directive.
export default {
  plugins: {
    autoprefixer: {},
  },
};
