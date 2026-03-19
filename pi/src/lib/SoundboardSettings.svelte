<script lang="ts">
	import { actionSettings, eventTarget, sendToPlugin } from "@openaction/svelte-pi";
	import { onDestroy, onMount } from "svelte";
	import SoundboardSoundPicker from "./SoundboardSoundPicker.svelte";

	interface GuildInfo {
		guild_id: string;
		name: string;
	}

	interface GuildsResponse {
		action: string;
		guilds: GuildInfo[];
		error?: string;
	}

	function parseGuildsResponse(detail: unknown): GuildsResponse | null {
		const candidate =
			detail && typeof detail === "object" && "payload" in detail
				? (detail as { payload: unknown }).payload
				: detail;

		if (typeof candidate === "string") {
			try {
				return JSON.parse(candidate) as GuildsResponse;
			} catch {
				return null;
			}
		}
		if (candidate && typeof candidate === "object") {
			return candidate as GuildsResponse;
		}
		return null;
	}

	let soundId = "";
	let guildId = "";
	let soundName = "";
	let guilds: GuildInfo[] = [];
	let loadingGuilds = false;
	let guildsError: string | null = null;
	let listener: ((event: Event) => void) | null = null;
	let guildRequestToken = 0;
	let soundRefreshToken = 0;

	$: {
		if ($actionSettings.sound_id !== undefined) {
			soundId = $actionSettings.sound_id;
		}
		if ($actionSettings.guild_id !== undefined) {
			guildId = $actionSettings.guild_id;
		}
		if ($actionSettings.sound_name !== undefined) {
			soundName = $actionSettings.sound_name;
		}
	}

	function handleEvent(event: Event) {
		const detail = parseGuildsResponse((event as CustomEvent<unknown>).detail);
		if (detail?.action !== "guilds_result") {
			return;
		}

		guilds = detail.guilds ?? [];
		loadingGuilds = false;
		guildsError = detail.error ?? null;
		if (!detail.error && guildId.trim()) {
			soundRefreshToken += 1;
		}
	}

	onMount(() => {
		listener = handleEvent;
		eventTarget.addEventListener("sendToPropertyInspector", listener);
		void refreshGuilds();
	});

	onDestroy(() => {
		if (listener) {
			eventTarget.removeEventListener("sendToPropertyInspector", listener);
		}
	});

	async function refreshGuilds() {
		const requestToken = ++guildRequestToken;
		loadingGuilds = true;
		guildsError = null;
		sendToPlugin({ action: "get_guilds" });
		window.setTimeout(() => {
			if (requestToken === guildRequestToken && loadingGuilds) {
				loadingGuilds = false;
				guildsError = "Discord server list timed out. Check plugin logs or use Manual Entry below.";
			}
		}, 16000);
	}

	function saveSettings() {
		sendToPlugin({
			sound_id: soundId,
			guild_id: guildId,
			sound_name: soundName,
		});
	}

	function handleGuildChange() {
		soundId = "";
		soundName = "";
		saveSettings();
	}
</script>

<div class="p-3">
	<h3 class="mb-3 text-sm font-semibold text-neutral-100">Soundboard Sound</h3>
	<p class="mb-3 text-xs text-neutral-400">
		Play a sound from your Discord soundboard. You need Nitro to use custom sounds.
	</p>

	<div class="mb-4 rounded-lg border border-neutral-600 bg-neutral-800 p-3">
		<h4 class="mb-2 text-xs font-semibold text-neutral-300">Quick Select</h4>
		<p class="mb-2 text-xs text-neutral-400">
			Choose a server and then select a sound from the dropdown. Make sure you're in a voice channel in that server.
		</p>

		<div class="mb-3">
			<div class="mb-1 flex items-center justify-between">
				<label for="guildIdPicker" class="block text-xs text-neutral-200">Server</label>
				<button
					type="button"
					on:click={refreshGuilds}
					disabled={loadingGuilds}
					class="text-xs text-neutral-400 hover:text-neutral-200 disabled:cursor-not-allowed disabled:opacity-50"
				>
					{#if loadingGuilds}
						Refreshing...
					{:else}
						Refresh
					{/if}
				</button>
			</div>
			<select
				id="guildIdPicker"
				bind:value={guildId}
				on:change={handleGuildChange}
				disabled={loadingGuilds || guilds.length === 0}
				class="w-full appearance-none rounded-lg border border-neutral-600 px-2 py-1 text-xs focus:border-neutral-600 focus:outline-none"
				style="background-color: rgb(64 64 64); color: rgb(245 245 245); color-scheme: dark;"
			>
				<option value="" style="background-color: rgb(38 38 38); color: rgb(245 245 245);">
					Select a server...
				</option>
				{#each guilds as guild}
					<option
						value={guild.guild_id}
						style="background-color: rgb(38 38 38); color: rgb(245 245 245);"
					>
						{guild.name}
					</option>
				{/each}
			</select>
			{#if guildsError}
				<p class="mt-1 text-xs text-amber-400">{guildsError}</p>
			{:else if !loadingGuilds && guilds.length === 0}
				<p class="mt-1 text-xs text-neutral-500">No servers returned by Discord.</p>
			{/if}
		</div>

		<SoundboardSoundPicker
			{guildId}
			{soundRefreshToken}
			bind:selectedSoundId={soundId}
			bind:selectedSoundName={soundName}
		/>
	</div>

	<div class="mb-4 border-t border-neutral-700 pt-4">
		<h4 class="mb-2 text-xs font-semibold text-neutral-300">Manual Entry (Advanced)</h4>
		<p class="mb-3 text-xs text-neutral-500">
			Only use these if the quick select above doesn't work or you're configuring a sound from a different server.
		</p>

		<div class="mb-3">
			<label for="soundId" class="mb-1 block text-xs text-neutral-200">Sound ID</label>
			<input
				id="soundId"
				type="text"
				bind:value={soundId}
				on:change={saveSettings}
				placeholder="Discord sound ID (snowflake)"
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 placeholder-neutral-500 focus:border-neutral-600 focus:outline-none"
			/>
		</div>

		<div class="mb-3">
			<label for="guildId" class="mb-1 block text-xs text-neutral-200">Guild ID</label>
			<input
				id="guildId"
				type="text"
				bind:value={guildId}
				on:change={saveSettings}
				placeholder="Server ID (snowflake)"
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 placeholder-neutral-500 focus:border-neutral-600 focus:outline-none"
			/>
		</div>

		<div class="mb-3">
			<label for="soundName" class="mb-1 block text-xs text-neutral-200">Sound Name</label>
			<input
				id="soundName"
				type="text"
				bind:value={soundName}
				on:change={saveSettings}
				placeholder="Display name for the sound"
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 placeholder-neutral-500 focus:border-neutral-600 focus:outline-none"
			/>
		</div>
	</div>

	{#if soundName}
		<div class="rounded-lg border border-green-600 bg-green-900/30 p-2 text-xs text-green-400">
			Selected: {soundName}
		</div>
	{/if}
</div>
