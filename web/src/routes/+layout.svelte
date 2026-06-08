<script lang="ts">
	import '../app.css';
	import '$lib/design.css';
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { jwt, tunnels, nodes, logout } from '$lib/stores';
	import { connectEvents, getNodes, getTunnels, type BhEvent } from '$lib/api';

	// Well-log navigation: each section sits at a "depth" like a borehole log.
	const nav = [
		{ depth: '0000', label: 'tunnels', href: '/' },
		{ depth: '0100', label: 'nodes', href: '/nodes' },
		{ depth: '0200', label: 'devices', href: '/devices' },
		{ depth: '0300', label: 'tokens', href: '/tokens' },
		{ depth: '0400', label: 'history', href: '/history' }
	];

	$: pathname = $page.url.pathname;
	$: section = pathname === '/' ? 'tunnels' : pathname.replace(/^\//, '');
	$: isLogin = pathname === '/login';

	let disconnect: (() => void) | null = null;

	/** Loads the initial dataset; a rejected token forces re-login. */
	async function loadAll(token: string) {
		try {
			const [t, n] = await Promise.all([getTunnels(token), getNodes(token)]);
			tunnels.set(t);
			nodes.set(n);
		} catch {
			logout();
		}
	}

	/** Applies a live event to the stores. */
	function handleEvent(token: string, e: BhEvent) {
		if (e.type === 'tunnel_opened') {
			tunnels.update((list) => [
				...list.filter((t) => t.tunnel_id !== e.tunnel_id),
				{
					tunnel_id: e.tunnel_id,
					protocol: e.protocol,
					local_port: e.local_port,
					remote_port: e.remote_port,
					node: e.node || null,
					device: null,
					started_at: e.started_at
				}
			]);
		} else if (e.type === 'tunnel_closed') {
			tunnels.update((list) => list.filter((t) => t.tunnel_id !== e.tunnel_id));
		} else if (e.type === 'node_connected' || e.type === 'node_left') {
			// Node events carry partial data; refetch the full list to stay exact.
			getNodes(token)
				.then((n) => nodes.set(n))
				.catch(() => {});
		}
	}

	/** (Re)establishes the data feed for a given token. */
	function connect(token: string) {
		disconnect?.();
		loadAll(token);
		disconnect = connectEvents(token, (e) => handleEvent(token, e));
	}

	onMount(() => {
		// React to auth changes: guard routes, wire/tear down the live feed.
		const unsub = jwt.subscribe((token) => {
			if (!browser) return;
			if (!token) {
				disconnect?.();
				disconnect = null;
				tunnels.set([]);
				nodes.set([]);
				if (pathname !== '/login') goto('/login');
			} else {
				if (pathname === '/login') goto('/');
				connect(token);
			}
		});
		return () => {
			unsub();
			disconnect?.();
		};
	});
</script>

{#if isLogin}
	<slot />
{:else}
	<div class="shell">
	<aside class="rail">
		<div class="brand">
			<span class="wordmark">borehole</span>
			<span class="ver">v2</span>
		</div>

		<nav class="well" aria-label="Sections">
			{#each nav as item (item.href)}
				{@const active = pathname === item.href}
				<a class="log" class:active href={item.href} aria-current={active ? 'page' : undefined}>
					<span class="mark">{active ? '▽' : '·'}</span>
					<span class="depth">{item.depth}</span>
					<span class="label">{item.label}</span>
				</a>
			{/each}
		</nav>

		<div class="ruler" aria-hidden="true"></div>
	</aside>

	<main class="main">
		<header class="topbar">
			<span class="crumb">borehole</span>
			<span class="sep">/</span>
			<span class="here">{section}</span>
		</header>
		<section class="content">
			<slot />
		</section>
	</main>
	</div>
{/if}

<style>
	.shell {
		display: grid;
		grid-template-columns: 208px 1fr;
		min-height: 100vh;
	}

	/* Left "well-log" rail */
	.rail {
		position: relative;
		background: var(--bh-ink2);
		border-right: 1px solid var(--bh-line2);
		padding: 18px 0 0 0;
	}

	.brand {
		display: flex;
		align-items: baseline;
		gap: 8px;
		padding: 0 20px 18px;
		border-bottom: 1px solid var(--bh-line2);
	}

	.wordmark {
		font-family: var(--bh-disp);
		font-weight: 800;
		font-size: 15px;
		letter-spacing: -0.01em;
		color: var(--bh-fg);
	}

	.ver {
		font-size: 9px;
		letter-spacing: 0.14em;
		color: var(--bh-faint);
		text-transform: uppercase;
	}

	.well {
		display: flex;
		flex-direction: column;
		padding: 12px 0;
	}

	.log {
		display: grid;
		grid-template-columns: 16px 40px 1fr;
		align-items: center;
		gap: 8px;
		padding: 7px 20px;
		color: var(--bh-dim);
		text-decoration: none;
		border-left: 2px solid transparent;
	}

	.log:hover {
		color: var(--bh-fg);
		background: var(--bh-blue-dim);
	}

	.log.active {
		color: var(--bh-fg);
		border-left-color: var(--bh-blue);
		background: var(--bh-blue-dim);
	}

	.mark {
		color: var(--bh-faint);
		font-size: 11px;
		text-align: center;
	}

	.log.active .mark {
		color: var(--bh-blue);
	}

	.depth {
		color: var(--bh-faint);
		font-size: 11px;
		font-variant-numeric: tabular-nums;
		letter-spacing: 0.06em;
	}

	.label {
		font-size: 12px;
		letter-spacing: 0.02em;
	}

	/* Depth ruler: a hairline band of evenly spaced ticks down the right edge */
	.ruler {
		position: absolute;
		top: 0;
		right: 0;
		bottom: 0;
		width: 14px;
		border-left: 1px solid var(--bh-line2);
		background-image: repeating-linear-gradient(
			to bottom,
			var(--bh-line) 0,
			var(--bh-line) 1px,
			transparent 1px,
			transparent 11px
		);
	}

	/* Main column */
	.main {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}

	.topbar {
		display: flex;
		align-items: center;
		gap: 8px;
		height: 44px;
		padding: 0 24px;
		border-bottom: 1px solid var(--bh-line2);
		font-size: 11px;
		letter-spacing: 0.1em;
		text-transform: uppercase;
	}

	.crumb {
		color: var(--bh-faint);
	}

	.sep {
		color: var(--bh-faint);
	}

	.here {
		color: var(--bh-fg);
	}

	.content {
		padding: 24px;
		flex: 1;
	}
</style>
