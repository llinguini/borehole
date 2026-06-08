<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { get } from 'svelte/store';
	import { closeTunnel, type Tunnel } from '$lib/api';
	import { jwt, tunnels } from '$lib/stores';

	// A ticking clock so uptime re-renders every second.
	let now = Date.now();
	let timer: ReturnType<typeof setInterval>;
	onMount(() => {
		timer = setInterval(() => (now = Date.now()), 1000);
	});
	onDestroy(() => clearInterval(timer));

	/** Tracks rows being closed to disable their button. */
	let closing = new Set<string>();

	async function onClose(id: string) {
		const token = get(jwt);
		if (!token || closing.has(id)) return;
		closing = new Set(closing).add(id);
		try {
			await closeTunnel(token, id);
			// Optimistic removal; the SSE `tunnel_closed` also filters it out.
			tunnels.update((list) => list.filter((t) => t.tunnel_id !== id));
		} catch {
			closing = new Set([...closing].filter((x) => x !== id));
		}
	}

	/** HH:MM:SS uptime from the RFC3339 open time. */
	function uptime(startedAt: string, clock: number): string {
		const start = Date.parse(startedAt);
		if (Number.isNaN(start)) return '—';
		const secs = Math.max(0, Math.floor((clock - start) / 1000));
		const h = Math.floor(secs / 3600);
		const m = Math.floor((secs % 3600) / 60);
		const s = secs % 60;
		return [h, m, s].map((n) => String(n).padStart(2, '0')).join(':');
	}

	const shortId = (id: string) => id.slice(0, 8);
	const nodeLabel = (t: Tunnel) => (t.node && t.node.length > 0 ? t.node : 'direct');
</script>

<div class="panel">
	<div class="head">
		<span class="title">tunnels</span>
		<span class="count">{$tunnels.length} active</span>
	</div>

	<table>
		<thead>
			<tr>
				<th>ID</th>
				<th>Proto</th>
				<th class="num">Local</th>
				<th class="num">Remoto</th>
				<th>Nodo</th>
				<th>Dispositivo</th>
				<th class="num">Uptime</th>
				<th class="act"></th>
			</tr>
		</thead>
		<tbody>
			{#each $tunnels as t (t.tunnel_id)}
				<tr>
					<td class="id">
						<span class="dot up"></span>
						{shortId(t.tunnel_id)}
					</td>
					<td class="proto">{t.protocol}</td>
					<td class="num">{t.local_port === null ? '—' : `:${t.local_port}`}</td>
					<td class="num">:{t.remote_port}</td>
					<td>{nodeLabel(t)}</td>
					<td class="dim">{t.device ?? '—'}</td>
					<td class="num">{uptime(t.started_at, now)}</td>
					<td class="act">
						<button
							class="x"
							title="Cerrar túnel"
							disabled={closing.has(t.tunnel_id)}
							on:click={() => onClose(t.tunnel_id)}
						>
							✕
						</button>
					</td>
				</tr>
			{:else}
				<tr>
					<td class="empty" colspan="8">Sin túneles activos</td>
				</tr>
			{/each}
		</tbody>
	</table>
</div>

<style>
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
		color: var(--bh-fg);
		border-bottom: 1px solid var(--bh-line);
		white-space: nowrap;
	}

	tbody tr:last-child td {
		border-bottom: none;
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
	}

	.proto {
		color: var(--bh-dim);
		text-transform: uppercase;
		font-size: 10px;
		letter-spacing: 0.12em;
	}

	.dim {
		color: var(--bh-dim);
	}

	.empty {
		text-align: center;
		color: var(--bh-faint);
		padding: 20px 16px;
	}

	/* 6px square status dot; active tunnels carry a subtle glow. */
	.dot {
		display: inline-block;
		width: 6px;
		height: 6px;
		margin-right: 9px;
		vertical-align: middle;
		background: var(--bh-faint);
	}

	.dot.up {
		background: var(--bh-up);
		box-shadow: 0 0 6px var(--bh-up);
	}

	.act {
		width: 32px;
		text-align: center;
	}

	.x {
		background: transparent;
		border: none;
		color: var(--bh-faint);
		font-family: var(--bh-mono);
		font-size: 12px;
		cursor: pointer;
		padding: 2px 6px;
	}

	.x:hover:not(:disabled) {
		color: var(--bh-down);
	}

	.x:disabled {
		color: var(--bh-line2);
		cursor: default;
	}
</style>
