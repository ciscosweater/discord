<script lang="ts">
	import { sendToPlugin } from "@openaction/svelte-pi";

	let soundId = "";
	let guildId = "";
	let soundName = "";

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
