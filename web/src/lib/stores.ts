// Svelte stores backing the dashboard.
//
// `jwt` is the single source of auth truth and is mirrored to localStorage so a
// reload keeps the session (until the token expires). `tunnels`/`nodes` hold the
// live dataset, kept in sync by the layout (initial fetch + SSE).

import { writable } from 'svelte/store';
import { browser } from '$app/environment';
import type { Node, Tunnel } from './api';

const STORAGE_KEY = 'borehole.jwt';

/** Reads a non-expired token from localStorage, or `null`. */
function initialJwt(): string | null {
	if (!browser) return null;
	const stored = localStorage.getItem(STORAGE_KEY);
	if (!stored) return null;
	return isExpired(stored) ? null : stored;
}

export const jwt = writable<string | null>(initialJwt());
export const tunnels = writable<Tunnel[]>([]);
export const nodes = writable<Node[]>([]);

// Persist the token across reloads; clearing it removes the stored value.
if (browser) {
	jwt.subscribe((value) => {
		if (value) localStorage.setItem(STORAGE_KEY, value);
		else localStorage.removeItem(STORAGE_KEY);
	});
}

/** Clears the session (used on logout or when the server rejects the token). */
export function logout() {
	jwt.set(null);
}

/**
 * Returns whether a JWT is expired by decoding its `exp` claim. Malformed
 * tokens are treated as expired (fail-closed).
 */
export function isExpired(token: string): boolean {
	try {
		const payload = token.split('.')[1];
		const json = atob(payload.replace(/-/g, '+').replace(/_/g, '/'));
		const data = JSON.parse(json) as { exp?: number };
		if (typeof data.exp !== 'number') return false;
		return data.exp * 1000 <= Date.now();
	} catch {
		return true;
	}
}
