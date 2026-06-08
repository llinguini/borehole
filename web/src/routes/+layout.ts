// Single-page app served by borehole-server. We disable SSR/prerendering and
// rely on adapter-static's `fallback` (index.html): the server embeds the build
// and serves index.html for any client route, where the SPA takes over.
//
// The auth guard, initial data fetch and SSE subscription live in
// `+layout.svelte` (`onMount`, browser-only) — the correct place for
// client-session logic that needs `localStorage` and a live server.
export const ssr = false;
export const prerender = false;
