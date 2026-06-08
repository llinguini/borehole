<script lang="ts">
	import { nodes } from '$lib/stores';

	// Load bar scaled against the busiest node (min 1 to avoid /0), so the bar is
	// relative within the current fleet.
	$: maxTunnels = Math.max(1, ...$nodes.map((n) => n.tunnel_count));
</script>

<div class="panel">
	<div class="head">
		<span class="title">nodes</span>
		<span class="count">{$nodes.length} connected</span>
	</div>

	<table>
		<thead>
			<tr>
				<th>Node</th>
				<th>Host</th>
				<th>Túneles activos</th>
				<th>Estado</th>
			</tr>
		</thead>
		<tbody>
			{#each $nodes as n (n.node_id)}
				<tr>
					<td class="name">{n.name}</td>
					<td class="dim">{n.host}</td>
					<td class="load">
						<span class="n">{n.tunnel_count}</span>
						<span class="bar">
							<span
								class="fill"
								style="width: {(n.tunnel_count / maxTunnels) * 100}%"
							></span>
						</span>
					</td>
					<td>
						<span class="dot up"></span>
						<span class="status">{n.status}</span>
					</td>
				</tr>
			{:else}
				<tr>
					<td class="empty" colspan="4">Sin nodos conectados</td>
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
		vertical-align: middle;
	}

	tbody tr:last-child td {
		border-bottom: none;
	}

	tbody tr:hover td {
		background: var(--bh-blue-dim);
	}

	.name {
		color: var(--bh-blue);
	}

	.dim {
		color: var(--bh-dim);
	}

	.load {
		display: flex;
		align-items: center;
		gap: 10px;
	}

	.load .n {
		font-variant-numeric: tabular-nums;
		min-width: 18px;
		text-align: right;
	}

	/* Load bar: hairline track + blue fill, 6px tall, no radius. */
	.bar {
		display: inline-block;
		width: 140px;
		height: 6px;
		background: var(--bh-ink3);
		border: 1px solid var(--bh-line);
	}

	.fill {
		display: block;
		height: 100%;
		background: var(--bh-blue);
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

	.status {
		text-transform: uppercase;
		font-size: 10px;
		letter-spacing: 0.12em;
		color: var(--bh-dim);
	}

	.empty {
		text-align: center;
		color: var(--bh-faint);
		padding: 20px 16px;
	}
</style>
