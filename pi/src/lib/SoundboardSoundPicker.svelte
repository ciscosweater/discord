<script lang="ts">
	import { onMount, onDestroy } from "svelte";
	import { sendToPlugin, eventTarget } from "@openaction/svelte-pi";

	interface SoundInfo {
		sound_id: string;
		name: string;
		emoji_name: string | null;
	}

	interface SoundsResponse {
		action: string;
		sounds: SoundInfo[];
		error?: string;
	}

	function parseSoundsResponse(detail: unknown): SoundsResponse | null {
		const candidate =
			detail && typeof detail === "object" && "payload" in detail
				? (detail as { payload: unknown }).payload
				: detail;

		if (typeof candidate === "string") {
			try {
				return JSON.parse(candidate) as SoundsResponse;
			} catch {
				return null;
			}
		}
		if (candidate && typeof candidate === "object") {
			return candidate as SoundsResponse;
		}
		return null;
	}

	export let guildId: string = "";
	export let soundRefreshToken: number = 0;
	export let selectedSoundId: string = "";
	export let selectedSoundName: string = "";

	let sounds: SoundInfo[] = [];
	let loading = false;
	let error: string | null = null;
	let listener: ((event: Event) => void) | null = null;
	let requestedGuildId = "";
	let initialized = false;
	let soundRequestToken = 0;
	let appliedRefreshToken = -1;

	function handleResponse(data: SoundsResponse) {
		if (data.action === "sounds_result") {
			sounds = data.sounds;
			loading = false;
			if (data.error) {
				error = data.error;
			} else if (sounds.length === 0 && guildId === requestedGuildId) {
				error = "No soundboard sounds found for this server";
			} else {
				error = null;
			}

			if (selectedSoundId && !sounds.some((sound) => sound.sound_id === selectedSoundId)) {
				selectedSoundId = "";
				selectedSoundName = "";
			}

			if (!selectedSoundName && selectedSoundId) {
				const selected = sounds.find((sound) => sound.sound_id === selectedSoundId);
				if (selected) {
					selectedSoundName = selected.name;
				}
			}
		}
	}

	function handleEvent(event: Event) {
		const detail = parseSoundsResponse((event as CustomEvent<unknown>).detail);
		if (detail?.action === "sounds_result") {
			handleResponse(detail);
		}
	}

	onMount(() => {
		initialized = true;
		listener = handleEvent;
		eventTarget.addEventListener("sendToPropertyInspector", listener);

		if (guildId) {
			requestSounds();
		}
	});

	onDestroy(() => {
		if (listener) {
			eventTarget.removeEventListener("sendToPropertyInspector", listener);
		}
	});

	$: if (initialized) {
		const normalizedGuildId = guildId.trim();
		if (!normalizedGuildId) {
			requestedGuildId = "";
			sounds = [];
			loading = false;
			error = "Enter a server ID to see available sounds";
		} else if (normalizedGuildId !== requestedGuildId) {
			void requestSounds();
		} else if (soundRefreshToken !== appliedRefreshToken) {
			appliedRefreshToken = soundRefreshToken;
			requestedGuildId = "";
			void requestSounds();
		}
	}

	async function requestSounds() {
		const normalizedGuildId = guildId.trim();
		if (!normalizedGuildId) {
			error = "Enter a server ID to see available sounds";
			sounds = [];
			loading = false;
			return;
		}

		const requestToken = ++soundRequestToken;
		requestedGuildId = normalizedGuildId;
		loading = true;
		sounds = [];
		error = null;
		sendToPlugin({
			action: "get_sounds",
			guild_id: normalizedGuildId,
		});
		window.setTimeout(() => {
			if (requestToken === soundRequestToken && loading) {
				loading = false;
				error = "Loading sounds timed out. Check plugin logs or use Manual Entry below.";
			}
		}, 16000);
	}

	function handleSoundSelect() {
		const selected = sounds.find((s) => s.sound_id === selectedSoundId);
		if (selected) {
			selectedSoundName = selected.name;
		} else {
			selectedSoundName = "";
		}

		// Save settings when a sound is selected
		sendToPlugin({
			sound_id: selectedSoundId,
			guild_id: requestedGuildId || guildId.trim(),
			sound_name: selectedSoundName,
		});
	}

	function handleRefresh() {
		requestedGuildId = "";
		void requestSounds();
	}
</script>

<div class="mb-3">
	<div class="mb-1 flex items-center justify-between">
		<label for="soundPicker" class="text-xs text-neutral-200">
			Sound
		</label>
		<button
			type="button"
			on:click={handleRefresh}
			disabled={!guildId || loading}
			class="text-xs text-neutral-400 hover:text-neutral-200 disabled:cursor-not-allowed disabled:opacity-50"
		>
			{#if loading}
				Refreshing...
			{:else}
				Refresh
			{/if}
		</button>
	</div>

	{#if loading && sounds.length === 0}
		<div class="flex items-center rounded-lg border border-neutral-600 bg-neutral-700 px-3 py-2 text-xs text-neutral-400">
			<svg class="mr-2 h-4 w-4 animate-spin" viewBox="0 0 24 24">
				<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" fill="none" />
				<path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
			</svg>
			Loading sounds...
		</div>
	{:else if error}
		<div class="rounded-lg border border-neutral-600 bg-neutral-700 px-3 py-2 text-xs text-neutral-400">
			{error}
			{#if !guildId}
				<p class="mt-1 text-neutral-500">Enter a server ID to see available sounds</p>
			{/if}
		</div>
	{:else if sounds.length > 0}
		<select
			id="soundPicker"
			bind:value={selectedSoundId}
			on:change={handleSoundSelect}
			class="w-full appearance-none rounded-lg border border-neutral-600 px-2 py-1.5 text-xs focus:border-neutral-600 focus:outline-none"
			style="background-color: rgb(64 64 64); color: rgb(245 245 245); color-scheme: dark;"
		>
			<option value="" style="background-color: rgb(38 38 38); color: rgb(245 245 245);">
				Select a sound...
			</option>
			{#each sounds as sound}
				<option
					value={sound.sound_id}
					style="background-color: rgb(38 38 38); color: rgb(245 245 245);"
				>
					{sound.emoji_name ? `${sound.emoji_name} ` : ""}{sound.name}
				</option>
			{/each}
		</select>
		<p class="mt-1 text-xs text-neutral-500">{sounds.length} sound{sounds.length !== 1 ? "s" : ""} available</p>
	{:else}
		<div class="rounded-lg border border-neutral-600 bg-neutral-700 px-3 py-2 text-xs text-neutral-400">
			{#if guildId}
				No sounds found for this server. Click Refresh to try again.
			{:else}
				Enter a server ID above to see available sounds
			{/if}
		</div>
	{/if}
</div>
