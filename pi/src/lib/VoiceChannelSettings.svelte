<script lang="ts">
	import { actionSettings, eventTarget, sendToPlugin } from "@openaction/svelte-pi";
	import { onDestroy, onMount } from "svelte";

	interface GuildInfo {
		guild_id: string;
		name: string;
		icon_hash?: string;
	}

	interface VoiceChannelInfo {
		channel_id: string;
		name: string;
	}

	interface GuildsResponse {
		action: string;
		guilds: GuildInfo[];
		error?: string;
	}

	interface ChannelsResponse {
		action: string;
		guild_id: string;
		channels: VoiceChannelInfo[];
		error?: string;
	}

	function parsePiPayload<T>(detail: unknown): T | null {
		const candidate =
			detail && typeof detail === "object" && "payload" in detail
				? (detail as { payload: unknown }).payload
				: detail;

		if (typeof candidate === "string") {
			try {
				return JSON.parse(candidate) as T;
			} catch {
				return null;
			}
		}
		if (candidate && typeof candidate === "object") {
			return candidate as T;
		}
		return null;
	}

	function parseBooleanSetting(value: unknown, defaultValue: boolean): boolean {
		if (typeof value === "boolean") {
			return value;
		}
		if (typeof value === "string") {
			const normalized = value.trim().toLowerCase();
			if (normalized === "true") {
				return true;
			}
			if (normalized === "false") {
				return false;
			}
		}
		return defaultValue;
	}

	let savedGuildId = "";
	let savedGuildIconHash = "";
	let savedChannelId = "";
	let savedChannelName = "";
	let savedShowChannelTitle = true;
	let draftGuildId = "";
	let draftChannelId = "";
	let draftChannelName = "";
	let showChannelTitle = true;
	let guilds: GuildInfo[] = [];
	let channels: VoiceChannelInfo[] = [];
	let loadingGuilds = false;
	let loadingChannels = false;
	let guildsError: string | null = null;
	let channelsError: string | null = null;
	let listener: ((event: Event) => void) | null = null;
	let guildRequestToken = 0;
	let channelRequestToken = 0;

	const selectStyle =
		"background-color: rgb(64 64 64); color: rgb(245 245 245); color-scheme: dark;";
	const optionStyle = "background-color: rgb(38 38 38); color: rgb(245 245 245);";

	$: {
		const nextSavedGuildId = $actionSettings.guild_id ?? "";
		const nextSavedGuildIconHash = $actionSettings.guild_icon_hash ?? "";
		const nextSavedChannelId = $actionSettings.channel_id ?? "";
		const nextSavedChannelName = $actionSettings.channel_name ?? "";
		const nextShowChannelTitle = parseBooleanSetting($actionSettings.show_channel_title, true);
		const changed =
			nextSavedGuildId !== savedGuildId ||
			nextSavedGuildIconHash !== savedGuildIconHash ||
			nextSavedChannelId !== savedChannelId ||
			nextSavedChannelName !== savedChannelName ||
			nextShowChannelTitle !== savedShowChannelTitle;

		if (changed) {
			savedGuildId = nextSavedGuildId;
			savedGuildIconHash = nextSavedGuildIconHash;
			savedChannelId = nextSavedChannelId;
			savedChannelName = nextSavedChannelName;
			savedShowChannelTitle = nextShowChannelTitle;
			showChannelTitle = nextShowChannelTitle;
			draftGuildId = nextSavedGuildId;
			draftChannelId = nextSavedChannelId;
			draftChannelName = nextSavedChannelName;
		}
	}

	$: selectedChannelMissing =
		!!savedChannelId &&
		channels.length > 0 &&
		!channels.some((channel) => channel.channel_id === savedChannelId);

	function handleGuildsResponse(response: GuildsResponse) {
		guilds = response.guilds ?? [];
		loadingGuilds = false;
		guildsError = response.error ?? null;
		if (!response.error && draftGuildId.trim()) {
			void refreshChannels(draftGuildId);
		}
	}

	function handleChannelsResponse(response: ChannelsResponse) {
		if (response.guild_id !== draftGuildId) {
			return;
		}
		channels = response.channels ?? [];
		loadingChannels = false;
		channelsError = response.error ?? null;
	}

	onMount(() => {
		listener = (event: Event) => {
			const detail = (event as CustomEvent<unknown>).detail;
			const guildsResponse = parsePiPayload<GuildsResponse>(detail);
			if (guildsResponse?.action === "guilds_result") {
				handleGuildsResponse(guildsResponse);
				return;
			}

			const channelsResponse = parsePiPayload<ChannelsResponse>(detail);
			if (channelsResponse?.action === "channels_result") {
				handleChannelsResponse(channelsResponse);
			}
		};

		eventTarget.addEventListener("sendToPropertyInspector", listener);
		void refreshGuilds();
	});

	onDestroy(() => {
		if (listener) {
			eventTarget.removeEventListener("sendToPropertyInspector", listener);
		}
	});

	function saveSettings() {
		sendToPlugin({
			guild_id: savedGuildId,
			guild_icon_hash: savedGuildIconHash,
			channel_id: savedChannelId,
			channel_name: savedChannelName,
			show_channel_title: showChannelTitle,
		});
	}

	function refreshGuilds() {
		const requestToken = ++guildRequestToken;
		loadingGuilds = true;
		guildsError = null;
		sendToPlugin({ action: "get_guilds" });
		window.setTimeout(() => {
			if (requestToken === guildRequestToken && loadingGuilds) {
				loadingGuilds = false;
				guildsError =
					"Discord server list timed out. Check plugin logs or use Manual Entry below.";
			}
		}, 16000);
	}

	function refreshChannels(guildId: string) {
		if (!guildId.trim()) {
			channels = [];
			channelsError = null;
			loadingChannels = false;
			return;
		}

		const requestToken = ++channelRequestToken;
		loadingChannels = true;
		channelsError = null;
		sendToPlugin({ action: "get_channels", guild_id: guildId });
		window.setTimeout(() => {
			if (requestToken === channelRequestToken && loadingChannels) {
				loadingChannels = false;
				channelsError =
					"Loading voice channels timed out. Check plugin logs and Discord RPC status.";
			}
		}, 16000);
	}

	function handleGuildChange() {
		draftChannelId = "";
		draftChannelName = "";
		void refreshChannels(draftGuildId);
	}

	function handleChannelChange() {
		const selected = channels.find((channel) => channel.channel_id === draftChannelId);
		const selectedGuild = guilds.find((guild) => guild.guild_id === draftGuildId);
		if (!selected) {
			return;
		}

		savedGuildId = draftGuildId;
		savedGuildIconHash = selectedGuild?.icon_hash ?? "";
		savedChannelId = selected.channel_id;
		savedChannelName = selected.name;
		draftChannelName = selected.name;
		saveSettings();
	}

	function handleManualGuildIdChange() {
		savedGuildIconHash = "";
		saveSettings();
	}
</script>

<div class="rounded-xl border border-neutral-700 bg-neutral-800 p-3 text-neutral-100 shadow-sm">
	<h3 class="mb-3 text-sm font-semibold text-neutral-100">Voice Channel</h3>
	<p class="mb-3 text-xs text-neutral-400">
		Join a specific Discord voice or stage channel.
	</p>

	<div class="mb-4 rounded-lg border border-neutral-700 bg-neutral-900/60 p-3">
		<div class="mb-1 flex items-center justify-between">
			<label for="guildPicker" class="block text-xs text-neutral-200">Server</label>
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

		<div class="relative mb-3">
			<select
				id="guildPicker"
				bind:value={draftGuildId}
				on:change={handleGuildChange}
				disabled={loadingGuilds || guilds.length === 0}
				class="w-full rounded-lg border border-neutral-600 px-2 py-1 pr-8 text-xs focus:border-neutral-600 focus:outline-none disabled:cursor-not-allowed disabled:opacity-60"
				style={selectStyle}
			>
				<option value="" style={optionStyle}>Select a server...</option>
				{#each guilds as guild}
					<option value={guild.guild_id} style={optionStyle}>{guild.name}</option>
				{/each}
			</select>
			<div class="pointer-events-none absolute inset-y-0 right-0 flex items-center pr-2 text-neutral-300">
				<svg class="h-4 w-4" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
					<path fill-rule="evenodd" d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z" clip-rule="evenodd" />
				</svg>
			</div>
		</div>

		{#if guildsError}
			<p class="mb-3 rounded-lg border border-amber-700 bg-amber-950/30 px-3 py-2 text-xs text-amber-300">
				{guildsError}
			</p>
		{/if}

		<div class="mb-1 flex items-center justify-between">
			<label for="channelPicker" class="block text-xs text-neutral-200">Voice Channel</label>
			<button
				type="button"
				on:click={() => refreshChannels(draftGuildId)}
				disabled={!draftGuildId || loadingChannels}
				class="text-xs text-neutral-400 hover:text-neutral-200 disabled:cursor-not-allowed disabled:opacity-50"
			>
				{#if loadingChannels}
					Refreshing...
				{:else}
					Refresh
				{/if}
			</button>
		</div>

		<div class="relative">
			<select
				id="channelPicker"
				bind:value={draftChannelId}
				on:change={handleChannelChange}
				disabled={!draftGuildId || loadingChannels || channels.length === 0}
				class="w-full rounded-lg border border-neutral-600 px-2 py-1 pr-8 text-xs focus:border-neutral-600 focus:outline-none disabled:cursor-not-allowed disabled:opacity-60"
				style={selectStyle}
			>
				<option value="" style={optionStyle}>Select a voice channel...</option>
				{#each channels as channel}
					<option value={channel.channel_id} style={optionStyle}>{channel.name}</option>
				{/each}
			</select>
			<div class="pointer-events-none absolute inset-y-0 right-0 flex items-center pr-2 text-neutral-300">
				<svg class="h-4 w-4" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
					<path fill-rule="evenodd" d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z" clip-rule="evenodd" />
				</svg>
			</div>
		</div>

		{#if channelsError}
			<p class="mt-2 rounded-lg border border-amber-700 bg-amber-950/30 px-3 py-2 text-xs text-amber-300">
				{channelsError}
			</p>
		{:else if selectedChannelMissing}
			<p class="mt-2 rounded-lg border border-amber-700 bg-amber-950/30 px-3 py-2 text-xs text-amber-300">
				The configured channel is not in the current channel list for this server.
			</p>
		{:else if draftGuildId && !loadingChannels && channels.length === 0}
			<p class="mt-2 rounded-lg border border-neutral-700 bg-neutral-800 px-3 py-2 text-xs text-neutral-400">
				No voice or stage channels returned for this server.
			</p>
		{/if}
	</div>

	{#if draftGuildId && draftGuildId !== savedGuildId}
		<div class="mb-4 rounded-lg border border-amber-700 bg-amber-950/30 p-2 text-xs text-amber-300">
			You are browsing channels from another server. This button keeps its current assignment until you select a new channel.
		</div>
	{/if}

	<div class="mb-4 rounded-lg border border-neutral-600 bg-neutral-800 p-3">
		<label for="showChannelTitle" class="flex cursor-pointer items-start gap-3">
			<input
				id="showChannelTitle"
				type="checkbox"
				bind:checked={showChannelTitle}
				on:change={saveSettings}
				class="mt-0.5 h-4 w-4 rounded border-neutral-500 bg-neutral-700 text-indigo-500 focus:ring-indigo-500"
			/>
			<div>
				<div class="text-xs font-semibold text-neutral-200">Show channel name on button</div>
				<p class="mt-1 text-xs text-neutral-400">
					Uses the Stream Deck title to show the selected voice channel name under the icon.
				</p>
			</div>
		</label>
	</div>

	<div class="mb-4 border-t border-neutral-700 pt-4">
		<h4 class="mb-2 text-xs font-semibold text-neutral-300">Manual Entry (Advanced)</h4>
		<p class="mb-3 text-xs text-neutral-500">
			Only use this if quick select does not work. Enable Developer Mode in Discord to copy channel IDs.
		</p>

		<div class="mb-3">
			<label for="guildId" class="mb-1 block text-xs text-neutral-200">Server ID</label>
			<input
				id="guildId"
				type="text"
				bind:value={savedGuildId}
				on:change={handleManualGuildIdChange}
				placeholder="Discord server ID (snowflake)"
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 placeholder-neutral-500 focus:border-neutral-600 focus:outline-none"
			/>
		</div>

		<div class="mb-3">
			<label for="channelName" class="mb-1 block text-xs text-neutral-200">Channel Name</label>
			<input
				id="channelName"
				type="text"
				bind:value={savedChannelName}
				on:change={saveSettings}
				placeholder="Display name shown on the button"
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 placeholder-neutral-500 focus:border-neutral-600 focus:outline-none"
			/>
		</div>

		<div class="mb-3">
			<label for="channelId" class="mb-1 block text-xs text-neutral-200">Channel ID</label>
			<input
				id="channelId"
				type="text"
				bind:value={savedChannelId}
				on:change={saveSettings}
				placeholder="Voice channel ID (snowflake)"
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 placeholder-neutral-500 focus:border-neutral-600 focus:outline-none"
			/>
		</div>
	</div>

	{#if savedChannelName}
		<div class="rounded-lg border border-green-600 bg-green-900/30 p-2 text-xs text-green-400">
			Selected: {savedChannelName}
		</div>
	{/if}
</div>
