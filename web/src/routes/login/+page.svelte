<script lang="ts">
	import { goto } from '$app/navigation';
	import { login } from '$lib/api';
	import { jwt } from '$lib/stores';

	let email = '';
	let password = '';
	let error = '';
	let busy = false;

	async function submit() {
		if (busy) return;
		busy = true;
		error = '';
		try {
			const res = await login(email, password);
			jwt.set(res.jwt);
			await goto('/');
		} catch {
			error = 'Credenciales no válidas';
		} finally {
			busy = false;
		}
	}
</script>

<div class="screen">
	<form class="panel card" on:submit|preventDefault={submit}>
		<div class="wordmark">borehole</div>

		<label for="email">Email</label>
		<input id="email" type="email" bind:value={email} autocomplete="username" required />

		<label for="password">Contraseña</label>
		<input
			id="password"
			type="password"
			bind:value={password}
			autocomplete="current-password"
			required
		/>

		{#if error}
			<p class="err">{error}</p>
		{/if}

		<button type="submit" disabled={busy}>
			{busy ? 'Entrando…' : 'Iniciar sesión'}
		</button>
	</form>
</div>

<style>
	.screen {
		display: flex;
		min-height: 100vh;
		align-items: center;
		justify-content: center;
		background: var(--bh-ink);
	}

	.card {
		display: flex;
		flex-direction: column;
		width: 320px;
		padding: 32px 28px;
		background: var(--bh-ink2);
	}

	.wordmark {
		font-family: var(--bh-disp);
		font-weight: 800;
		font-size: 20px;
		letter-spacing: -0.01em;
		text-align: center;
		color: var(--bh-fg);
		margin-bottom: 24px;
	}

	label {
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.18em;
		color: var(--bh-faint);
		margin: 14px 0 5px;
	}

	input {
		background: var(--bh-ink);
		border: 1px solid var(--bh-line2);
		color: var(--bh-fg);
		font-family: var(--bh-mono);
		font-size: 13px;
		padding: 9px 10px;
	}

	input:focus {
		outline: none;
		border-color: var(--bh-blue);
	}

	/* Outlined button per the profile: blue border + blue text, no solid fill. */
	button {
		margin-top: 24px;
		background: transparent;
		border: 1px solid var(--bh-blue);
		color: var(--bh-blue);
		font-family: var(--bh-mono);
		font-size: 12px;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		padding: 10px;
		cursor: pointer;
	}

	button:hover:not(:disabled) {
		background: var(--bh-blue-dim);
	}

	button:disabled {
		color: var(--bh-faint);
		border-color: var(--bh-line2);
		cursor: default;
	}

	.err {
		color: var(--bh-down);
		font-size: 11px;
		margin: 12px 0 0;
	}
</style>
