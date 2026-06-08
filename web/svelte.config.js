import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	preprocess: vitePreprocess(),
	kit: {
		// Static output in ./build. `fallback` makes it a SPA: the server embeds
		// these files and serves index.html for any unmatched (client) route.
		adapter: adapter({ pages: 'build', assets: 'build', fallback: 'index.html' }),
		paths: { base: '' }
	}
};

export default config;
