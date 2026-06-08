// Typed client for the borehole-server REST + SSE API.
//
// The base URL comes from VITE_API_URL (set at build/dev time), defaulting to
// the server's standard HTTP port. All authenticated calls take the JWT
// returned by `login`.

const BASE = import.meta.env.VITE_API_URL ?? 'http://localhost:8080';

/** A live tunnel as returned by `GET /api/tunnels`. */
export type Tunnel = {
	tunnel_id: string;
	protocol: string;
	/** `null` when only known from a live event (the event omits it). */
	local_port: number | null;
	remote_port: number;
	/** Edge node name, or `null`/empty for the direct path. */
	node: string | null;
	/** Device label; not carried by the protocol yet (always `null`). */
	device: string | null;
	/** RFC3339 open time; uptime is computed client-side. */
	started_at: string;
};

/** An edge node as returned by `GET /api/nodes`. */
export type Node = {
	node_id: string;
	name: string;
	host: string;
	data_port: number;
	tunnel_count: number;
	status: string;
};

/** An API token as returned by `GET /api/tokens` (no secret). */
export type ApiToken = {
	id: string;
	name: string;
	scope: string;
	created_at: string;
	revoked: boolean;
};

/** `POST /api/tokens` response: the full secret (shown once) plus the token. */
export type CreatedToken = ApiToken & { token: string };

/** A CLI device as returned by `GET /api/devices`. */
export type Device = {
	id: string;
	hostname: string;
	last_seen_at: string;
	tunnel_count: number;
};

/** A closed tunnel as returned by `GET /api/history/tunnels`. */
export type TunnelHistory = {
	tunnel_id: string;
	protocol: string;
	local_port: number;
	remote_port: number;
	node: string | null;
	started_at: string;
	ended_at: string;
	duration_secs: number;
};

/** A past connection as returned by `GET /api/history/connections`. */
export type ConnectionHistory = {
	tunnel_id: string;
	conn_id: string;
	source_ip: string;
	source_port: number;
	connected_at: string;
	duration_secs: number;
};

/** Real-time event pushed over SSE (`GET /api/events`). */
export type BhEvent =
	| {
			type: 'tunnel_opened';
			tunnel_id: string;
			protocol: string;
			local_port: number;
			remote_port: number;
			node: string;
			started_at: string;
	  }
	| { type: 'tunnel_closed'; tunnel_id: string }
	| { type: 'node_connected'; node_id: string; name: string }
	| { type: 'node_left'; node_id: string };

/** Authenticates and returns a signed JWT. Throws on bad credentials. */
export async function login(email: string, password: string) {
	const r = await fetch(`${BASE}/api/auth/login`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify({ email, password })
	});
	if (!r.ok) throw new Error(await r.text());
	return r.json() as Promise<{ jwt: string; expires_at: string }>;
}

/** Fetches the active tunnels. Throws on auth failure. */
export async function getTunnels(jwt: string) {
	const r = await fetch(`${BASE}/api/tunnels`, { headers: auth(jwt) });
	if (!r.ok) throw new Error(`GET /api/tunnels: ${r.status}`);
	return r.json() as Promise<Tunnel[]>;
}

/** Closes a tunnel by id. Throws on failure. */
export async function closeTunnel(jwt: string, id: string) {
	const r = await fetch(`${BASE}/api/tunnels/${encodeURIComponent(id)}`, {
		method: 'DELETE',
		headers: auth(jwt)
	});
	if (!r.ok) throw new Error(`DELETE /api/tunnels/${id}: ${r.status}`);
}

/** Fetches the connected edge nodes. Throws on auth failure. */
export async function getNodes(jwt: string) {
	const r = await fetch(`${BASE}/api/nodes`, { headers: auth(jwt) });
	if (!r.ok) throw new Error(`GET /api/nodes: ${r.status}`);
	return r.json() as Promise<Node[]>;
}

/** Lists the authenticated user's API tokens. Throws on auth failure. */
export async function getTokens(jwt: string) {
	const r = await fetch(`${BASE}/api/tokens`, { headers: auth(jwt) });
	if (!r.ok) throw new Error(`GET /api/tokens: ${r.status}`);
	return r.json() as Promise<ApiToken[]>;
}

/** Creates a token; the response carries the full secret exactly once. */
export async function createToken(jwt: string, name: string, scope: string) {
	const r = await fetch(`${BASE}/api/tokens`, {
		method: 'POST',
		headers: { ...auth(jwt), 'Content-Type': 'application/json' },
		body: JSON.stringify({ name, scope })
	});
	if (!r.ok) throw new Error(await r.text());
	return r.json() as Promise<CreatedToken>;
}

/** Revokes a token by id. Throws on failure. */
export async function revokeToken(jwt: string, id: string) {
	const r = await fetch(`${BASE}/api/tokens/${encodeURIComponent(id)}`, {
		method: 'DELETE',
		headers: auth(jwt)
	});
	if (!r.ok) throw new Error(`DELETE /api/tokens/${id}: ${r.status}`);
}

/** Fetches the known CLI devices. Throws on auth failure. */
export async function getDevices(jwt: string) {
	const r = await fetch(`${BASE}/api/devices`, { headers: auth(jwt) });
	if (!r.ok) throw new Error(`GET /api/devices: ${r.status}`);
	return r.json() as Promise<Device[]>;
}

/** Fetches closed-tunnel history (most recent first). Throws on auth failure. */
export async function getHistoryTunnels(jwt: string, limit = 50, offset = 0) {
	const url = `${BASE}/api/history/tunnels?limit=${limit}&offset=${offset}`;
	const r = await fetch(url, { headers: auth(jwt) });
	if (!r.ok) throw new Error(`GET /api/history/tunnels: ${r.status}`);
	return r.json() as Promise<TunnelHistory[]>;
}

/** Fetches connection history (optionally per tunnel). Throws on auth failure. */
export async function getHistoryConnections(jwt: string, tunnelId?: string, limit = 50) {
	const params = new URLSearchParams({ limit: String(limit) });
	if (tunnelId) params.set('tunnel_id', tunnelId);
	const r = await fetch(`${BASE}/api/history/connections?${params}`, { headers: auth(jwt) });
	if (!r.ok) throw new Error(`GET /api/history/connections: ${r.status}`);
	return r.json() as Promise<ConnectionHistory[]>;
}

/**
 * Subscribes to the server event stream. `EventSource` cannot set headers, so
 * the JWT travels as a query param (the server accepts `?token=` there).
 * Returns a cleanup function that closes the connection.
 */
export function connectEvents(jwt: string, onEvent: (e: BhEvent) => void) {
	const es = new EventSource(`${BASE}/api/events?token=${encodeURIComponent(jwt)}`);
	es.onmessage = (e) => {
		try {
			onEvent(JSON.parse(e.data) as BhEvent);
		} catch {
			// Ignore malformed frames (e.g. keep-alive comments never reach here).
		}
	};
	return () => es.close();
}

/** Builds the Bearer authorization header. */
function auth(jwt: string): HeadersInit {
	return { Authorization: `Bearer ${jwt}` };
}
