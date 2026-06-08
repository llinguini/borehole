# borehole web — dashboard (v2)

SvelteKit dashboard for borehole, styled with the borehole design profile
(IBM Plex Mono / Martian Mono, dark "well-log" aesthetic, hairlines, no radius).

## Develop

```bash
npm install
npm run dev      # http://localhost:5173
```

## Build

```bash
npm run build    # static output in ./build
npm run preview
```

## Layout

- `src/lib/design.css` — design tokens (colors, fonts) from the design profile.
- `src/app.css` — Tailwind layers + shared panel (corner ticks) helper.
- `src/routes/+layout.svelte` — left "well-log" nav rail, depth ruler, wordmark.
- `src/routes/+page.svelte` — the dense "tunnels" table (mock data for now).
