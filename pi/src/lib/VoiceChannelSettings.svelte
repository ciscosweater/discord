<script lang="ts">
	import { sendToPlugin } from "@openaction/svelte-pi";

	let channelId = "";

	function saveSettings() {
		sendToPlugin({
			channel_id: channelId,
		});
	}
</script>

<div class="p-3">
	<h3 class="mb-3 text-sm font-semibold text-neutral-100">Voice Channel</h3>
	<p class="mb-3 text-xs text-neutral-400">
		Join a specific voice channel. Enable Developer Mode in Discord to get channel IDs.
	</p>

	<div class="mb-3">
		<label for="channelId" class="mb-1 block text-xs text-neutral-200">Channel ID</label>
		<input
			id="channelId"
			type="text"
			bind:value={channelId}
			on:change={saveSettings}
			placeholder="Voice channel ID (snowflake)"
			class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 placeholder-neutral-500 focus:border-neutral-600 focus:outline-none"
		/>
	</div>
</div>
