<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { get } from 'svelte/store';
	import {
		getHistoryConnections,
		getHistoryTunnels,
		type ConnectionHistory,
		type TunnelHistory
	} from '$lib/api';
	import { jwt, logout, tunnels } from '$lib/stores';

	let closed: TunnelHistory[] = [];
	let connections: ConnectionHistory[] = [];
	let loading = true;

	// Ticking clock for live durations of still-active sessions.
	let now = Date.now();
	let timer: ReturnType<typeof setInterval>;

	onMount(() => {
		load();
		timer = setInterval(() => (now = Date.now()), 1000);
	});
	onDestroy(() => clearInterval(timer));

	async function load() {
		const token = get(jwt);
		if (!token) return;
		try {
			[closed, connections] = await Promise.all([
				getHistoryTunnels(token),
				getHistoryConnections(token)
			]);
		} catch {
			logout();
		} finally {
			loading = false;
		}
	}

	const shortId = (id: string) => id.slice(0, 8);
	const nodeLabel = (node: string | null) => (node && node.length > 0 ? node : 'direct');

	function fmtTime(iso: string): string {
		const d = new Date(iso);
		return Number.isNaN(d.getTime()) ? '—' : d.toLocaleTimeString();
	}

	/** HH:MM:SS from a whole-second duration. */
	function fmtDuration(secs: number): string {
		const s = Math.max(0, Math.floor(secs));
		const h = Math.floor(s / 3600);
		const m = Math.floor((s % 3600) / 60);
		return [h, m, s % 60].map((n) => String(n).padStart(2, '0')).join(':');
	}

	/** Live duration (seconds) since an RFC3339 start, against the clock. */
	function liveDuration(startedAt: string, clock: number): string {
		const start = Date.parse(startedAt);
		if (Number.isNaN(start)) return '—';
		return fmtDuration((clock - start) / 1000);
	}
</script>

<div class="panel">
	<div class="head">
		<span class="title">history · sesiones</span>
		<span class="count">{$tunnels.length} activas · {closed.length} cerradas</span>
	</div>

	<table>
		<thead>
			<tr>
				<th>ID</th>
				<th>Proto</th>
				<th class="num">Local</th>
				<th class="num">Remoto</th>
				<th>Nodo</th>
				<th>Inicio</th>
				<th>Fin</th>
				<th class="num">Duración</th>
			</tr>
		</thead>
		<tbody>
			{#each $tunnels as t (t.tunnel_id)}
				<tr class="active">
					<td class="id">{shortId(t.tunnel_id)}</td>
					<td class="proto">{t.protocol}</td>
					<td class="num">{t.local_port === null ? '—' : `:${t.local_port}`}</td>
					<td class="num">:{t.remote_port}</td>
					<td>{nodeLabel(t.node)}</td>
					<td class="dim">{fmtTime(t.started_at)}</td>
					<td class="dim">—</td>
					<td class="num">{liveDuration(t.started_at, now)}</td>
				</tr>
			{/each}
			{#each closed as h (h.tunnel_id)}
				<tr class="past">
					<td class="id">{shortId(h.tunnel_id)}</td>
					<td class="proto">{h.protocol}</td>
					<td class="num">:{h.local_port}</td>
					<td class="num">:{h.remote_port}</td>
					<td>{nodeLabel(h.node)}</td>
					<td>{fmtTime(h.started_at)}</td>
					<td>{fmtTime(h.ended_at)}</td>
					<td class="num">{fmtDuration(h.duration_secs)}</td>
				</tr>
			{/each}
			{#if $tunnels.length === 0 && closed.length === 0}
				<tr>
					<td class="empty" colspan="8">
						{loading ? 'Cargando…' : 'Sin sesiones'}
					</td>
				</tr>
			{/if}
		</tbody>
	</table>
</div>

<div class="panel gap">
	<div class="head">
		<span class="title">history · conexiones</span>
		<span class="count">{connections.length} recientes</span>
	</div>

	<table>
		<thead>
			<tr>
				<th>Túnel</th>
				<th>IP origen</th>
				<th class="num">Puerto</th>
				<th>Conectado</th>
				<th class="num">Duración</th>
			</tr>
		</thead>
		<tbody>
			{#each connections as c (c.conn_id)}
				<tr class="past">
					<td class="id">{shortId(c.tunnel_id)}</td>
					<td><span class="flag">—</span>{c.source_ip}</td>
					<td class="num">{c.source_port}</td>
					<td>{fmtTime(c.connected_at)}</td>
					<td class="num">{fmtDuration(c.duration_secs)}</td>
				</tr>
			{:else}
				<tr>
					<td class="empty" colspan="5">
						{loading ? 'Cargando…' : 'Sin conexiones'}
					</td>
				</tr>
			{/each}
		</tbody>
	</table>
</div>

<style>
	.gap {
		margin-top: 24px;
	}

	.head {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		padding: 12px 16px;
		border-bottom: 1px solid var(--bh-line2);
	}

	.title {
		font-family: var(--bh-disp);
		font-weight: 600;
		font-size: 12px;
		letter-spacing: 0.02em;
	}

	.count {
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.18em;
		color: var(--bh-faint);
	}

	table {
		width: 100%;
		border-collapse: collapse;
	}

	th {
		text-align: left;
		font-weight: 500;
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.18em;
		color: var(--bh-faint);
		padding: 10px 16px;
		border-bottom: 1px solid var(--bh-line2);
		white-space: nowrap;
	}

	td {
		padding: 9px 16px;
		font-size: 12px;
		border-bottom: 1px solid var(--bh-line);
		white-space: nowrap;
	}

	tbody tr:last-child td {
		border-bottom: none;
	}

	/* Active sessions read in full strength; closed/past rows are dimmed. */
	tr.active td {
		color: var(--bh-fg);
	}

	tr.past td {
		color: var(--bh-faint);
	}

	tbody tr:hover td {
		background: var(--bh-blue-dim);
	}

	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.id {
		color: var(--bh-blue);
		font-variant-numeric: tabular-nums;
	}

	tr.past .id {
		color: var(--bh-dim);
	}

	.proto {
		text-transform: uppercase;
		font-size: 10px;
		letter-spacing: 0.12em;
	}

	.dim {
		color: var(--bh-dim);
	}

	.flag {
		color: var(--bh-faint);
		margin-right: 8px;
	}

	.empty {
		text-align: center;
		color: var(--bh-faint);
		padding: 20px 16px;
	}
</style>
