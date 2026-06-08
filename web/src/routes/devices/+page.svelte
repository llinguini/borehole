<script lang="ts">
	import { onMount } from 'svelte';
	import { get } from 'svelte/store';
	import { getDevices, type Device } from '$lib/api';
	import { jwt, logout } from '$lib/stores';

	let devices: Device[] = [];
	let loading = true;

	onMount(load);

	async function load() {
		const token = get(jwt);
		if (!token) return;
		try {
			devices = await getDevices(token);
		} catch {
			logout();
		} finally {
			loading = false;
		}
	}

	function fmt(iso: string): string {
		const d = new Date(iso);
		return Number.isNaN(d.getTime()) ? '—' : d.toLocaleString();
	}
</script>

<div class="panel">
	<div class="head">
		<span class="title">devices</span>
		<span class="count">{devices.length} known</span>
	</div>

	<table>
		<thead>
			<tr>
				<th>Hostname</th>
				<th>Última conexión</th>
				<th class="num">Túneles activos</th>
			</tr>
		</thead>
		<tbody>
			{#each devices as d (d.id)}
				<tr>
					<td class="host">
						<span class="dot" class:up={d.tunnel_count > 0}></span>
						{d.hostname}
					</td>
					<td class="dim">{fmt(d.last_seen_at)}</td>
					<td class="num">{d.tunnel_count}</td>
				</tr>
			{:else}
				<tr>
					<td class="empty" colspan="3">
						{loading ? 'Cargando…' : 'Sin dispositivos'}
					</td>
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

	.host {
		color: var(--bh-fg);
	}

	.dim {
		color: var(--bh-dim);
	}

	.empty {
		text-align: center;
		color: var(--bh-faint);
		padding: 20px 16px;
	}

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
</style>
