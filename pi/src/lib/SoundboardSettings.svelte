<script lang="ts">
	import { actionSettings, sendToPlugin } from "@openaction/svelte-pi";
	import SoundboardSoundPicker from "./SoundboardSoundPicker.svelte";

	let soundId = "";
	let guildId = "";
	let soundName = "";

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

	function saveSettings() {
		sendToPlugin({
			sound_id: soundId,
			guild_id: guildId,
			sound_name: soundName,
		});
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
			Enter the Server ID and select a sound from the dropdown. Make sure you're in a voice channel in that server.
		</p>

		<div class="mb-3">
			<label for="guildIdPicker" class="mb-1 block text-xs text-neutral-200">Server ID</label>
			<input
				id="guildIdPicker"
				type="text"
				bind:value={guildId}
				on:change={saveSettings}
				placeholder="Enter server ID (snowflake)"
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 placeholder-neutral-500 focus:border-neutral-600 focus:outline-none"
			/>
		</div>

		<SoundboardSoundPicker
			{guildId}
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
