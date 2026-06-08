<script lang="ts">
	import { onMount } from 'svelte';
	import { get } from 'svelte/store';
	import { createToken, getTokens, revokeToken, type ApiToken } from '$lib/api';
	import { jwt, logout } from '$lib/stores';

	let tokens: ApiToken[] = [];
	let loading = true;

	// New-token form (inline, in normal flow — no fixed overlay).
	let showForm = false;
	let name = '';
	let scope = 'all';
	let creating = false;
	let formError = '';

	// The full secret is shown exactly once, right after creation.
	let createdSecret: string | null = null;

	onMount(load);

	async function load() {
		const token = get(jwt);
		if (!token) return;
		try {
			tokens = await getTokens(token);
		} catch {
			logout();
		} finally {
			loading = false;
		}
	}

	function openForm() {
		showForm = true;
		formError = '';
		createdSecret = null;
	}

	async function onCreate() {
		const token = get(jwt);
		if (!token || creating) return;
		if (name.trim() === '') {
			formError = 'El nombre es obligatorio.';
			return;
		}
		creating = true;
		formError = '';
		try {
			const created = await createToken(token, name.trim(), scope);
			createdSecret = created.token;
			const { token: _secret, ...info } = created;
			tokens = [info, ...tokens];
			name = '';
			scope = 'all';
			showForm = false;
		} catch (e) {
			formError = e instanceof Error ? e.message : 'No se pudo crear el token.';
		} finally {
			creating = false;
		}
	}

	let revoking = new Set<string>();

	async function onRevoke(id: string) {
		const token = get(jwt);
		if (!token || revoking.has(id)) return;
		revoking = new Set(revoking).add(id);
		try {
			await revokeToken(token, id);
			tokens = tokens.map((t) => (t.id === id ? { ...t, revoked: true } : t));
		} catch {
			// Leave the row as-is on failure.
		} finally {
			revoking = new Set([...revoking].filter((x) => x !== id));
		}
	}

	function fmt(iso: string): string {
		const d = new Date(iso);
		return Number.isNaN(d.getTime()) ? '—' : d.toLocaleString();
	}
</script>

<div class="panel">
	<div class="head">
		<span class="title">tokens</span>
		<div class="head-right">
			<span class="count">{tokens.length} total</span>
			<button class="new" on:click={openForm}>+ nuevo token</button>
		</div>
	</div>

	{#if createdSecret}
		<div class="reveal">
			<p class="reveal-label">Copia este token ahora — no se volverá a mostrar:</p>
			<code class="reveal-secret">{createdSecret}</code>
			<button class="dismiss" on:click={() => (createdSecret = null)}>entendido</button>
		</div>
	{/if}

	{#if showForm}
		<div class="form">
			<div class="field">
				<label for="tk-name">Nombre</label>
				<input id="tk-name" type="text" bind:value={name} placeholder="ci-deploy" />
			</div>
			<div class="field">
				<label for="tk-scope">Scope</label>
				<select id="tk-scope" bind:value={scope}>
					<option value="all">all</option>
					<option value="tcp">tcp</option>
					<option value="http">http</option>
				</select>
			</div>
			<div class="form-actions">
				<button class="primary" disabled={creating} on:click={onCreate}>
					{creating ? 'creando…' : 'crear'}
				</button>
				<button class="ghost" on:click={() => (showForm = false)}>cancelar</button>
			</div>
			{#if formError}<p class="err">{formError}</p>{/if}
		</div>
	{/if}

	<table>
		<thead>
			<tr>
				<th>ID</th>
				<th>Nombre</th>
				<th>Scope</th>
				<th>Creado</th>
				<th>Estado</th>
				<th class="act"></th>
			</tr>
		</thead>
		<tbody>
			{#each tokens as t (t.id)}
				<tr class:revoked={t.revoked}>
					<td class="id">{t.id}</td>
					<td>{t.name}</td>
					<td class="scope">{t.scope}</td>
					<td class="dim">{fmt(t.created_at)}</td>
					<td>
						{#if t.revoked}
							<span class="badge down">revocado</span>
						{:else}
							<span class="badge up">activo</span>
						{/if}
					</td>
					<td class="act">
						{#if !t.revoked}
							<button
								class="revoke"
								disabled={revoking.has(t.id)}
								on:click={() => onRevoke(t.id)}
							>
								revocar
							</button>
						{/if}
					</td>
				</tr>
			{:else}
				<tr>
					<td class="empty" colspan="6">
						{loading ? 'Cargando…' : 'Sin tokens'}
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

	.head-right {
		display: flex;
		align-items: center;
		gap: 16px;
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

	.new {
		background: transparent;
		border: 1px solid var(--bh-line2);
		color: var(--bh-blue);
		font-family: var(--bh-mono);
		font-size: 11px;
		padding: 5px 10px;
		cursor: pointer;
	}

	.new:hover {
		background: var(--bh-blue-dim);
	}

	/* Reveal banner: shows the freshly minted secret in normal flow. */
	.reveal {
		display: flex;
		align-items: center;
		gap: 12px;
		flex-wrap: wrap;
		padding: 12px 16px;
		border-bottom: 1px solid var(--bh-line2);
		background: var(--bh-blue-dim);
	}

	.reveal-label {
		margin: 0;
		font-size: 11px;
		color: var(--bh-fg);
	}

	.reveal-secret {
		font-family: var(--bh-mono);
		font-size: 12px;
		color: var(--bh-blue);
		background: var(--bh-ink2);
		border: 1px solid var(--bh-line2);
		padding: 4px 8px;
		user-select: all;
	}

	.dismiss {
		background: transparent;
		border: 1px solid var(--bh-line2);
		color: var(--bh-dim);
		font-family: var(--bh-mono);
		font-size: 11px;
		padding: 4px 10px;
		cursor: pointer;
	}

	/* Inline create form, normal flow. */
	.form {
		display: flex;
		align-items: flex-end;
		gap: 16px;
		flex-wrap: wrap;
		padding: 14px 16px;
		border-bottom: 1px solid var(--bh-line2);
	}

	.field {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.field label {
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.18em;
		color: var(--bh-faint);
	}

	input,
	select {
		background: var(--bh-ink2);
		border: 1px solid var(--bh-line2);
		color: var(--bh-fg);
		font-family: var(--bh-mono);
		font-size: 12px;
		padding: 6px 8px;
		min-width: 160px;
	}

	.form-actions {
		display: flex;
		gap: 8px;
	}

	.primary {
		background: var(--bh-blue);
		border: none;
		color: var(--bh-ink2);
		font-family: var(--bh-mono);
		font-size: 11px;
		font-weight: 600;
		padding: 7px 14px;
		cursor: pointer;
	}

	.primary:disabled {
		opacity: 0.6;
		cursor: default;
	}

	.ghost {
		background: transparent;
		border: 1px solid var(--bh-line2);
		color: var(--bh-dim);
		font-family: var(--bh-mono);
		font-size: 11px;
		padding: 7px 14px;
		cursor: pointer;
	}

	.err {
		flex-basis: 100%;
		margin: 0;
		font-size: 11px;
		color: var(--bh-down);
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

	tr.revoked td {
		color: var(--bh-faint);
		text-decoration: line-through;
	}

	.id {
		color: var(--bh-blue);
		font-variant-numeric: tabular-nums;
	}

	tr.revoked .id {
		color: var(--bh-faint);
	}

	.scope {
		text-transform: uppercase;
		font-size: 10px;
		letter-spacing: 0.12em;
		color: var(--bh-dim);
	}

	.dim {
		color: var(--bh-dim);
	}

	.badge {
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.14em;
	}

	.badge.up {
		color: var(--bh-up);
	}

	.badge.down {
		color: var(--bh-down);
	}

	.empty {
		text-align: center;
		color: var(--bh-faint);
		padding: 20px 16px;
	}

	.act {
		width: 80px;
		text-align: right;
	}

	.revoke {
		background: transparent;
		border: none;
		color: var(--bh-faint);
		font-family: var(--bh-mono);
		font-size: 11px;
		cursor: pointer;
		padding: 2px 6px;
	}

	.revoke:hover:not(:disabled) {
		color: var(--bh-down);
	}

	.revoke:disabled {
		color: var(--bh-line2);
		cursor: default;
	}
</style>
