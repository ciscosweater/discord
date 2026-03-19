<script lang="ts">
	import { onDestroy, onMount } from "svelte";
	import { actionSettings, eventTarget, sendToPlugin } from "@openaction/svelte-pi";

	interface UserInfo {
		user_id: string;
		name: string;
		nick?: string | null;
		volume: number;
		mute: boolean;
	}

	interface UsersResponse {
		action: string;
		users: UserInfo[];
	}

	let userId = "";
	let mode = "mute";
	let muteType = "toggle";
	let adjustValue = 0;
	let setValue = 100;
	let users: UserInfo[] = [];
	let listener: ((event: Event) => void) | null = null;

	$: {
		if ($actionSettings.user_id !== undefined) {
			userId = $actionSettings.user_id;
		}
		if ($actionSettings.mode !== undefined) {
			mode = $actionSettings.mode;
		}
		if ($actionSettings.mute_type !== undefined) {
			muteType = $actionSettings.mute_type;
		}
		if ($actionSettings.adjust_value !== undefined) {
			adjustValue = Number($actionSettings.adjust_value) || 0;
		}
		if ($actionSettings.set_value !== undefined) {
			setValue = Number($actionSettings.set_value) || 100;
		}
	}

	onMount(() => {
		listener = (event: Event) => {
			const detail = (event as CustomEvent<UsersResponse>).detail;
			if (detail?.action === "users_result") {
				users = detail.users;
			}
		};
		eventTarget.addEventListener("sendToPropertyInspector", listener);
		sendToPlugin({ action: "get_users" });
	});

	onDestroy(() => {
		if (listener) {
			eventTarget.removeEventListener("sendToPropertyInspector", listener);
		}
	});

	function saveSettings() {
		sendToPlugin({
			user_id: userId,
			mode,
			mute_type: muteType,
			adjust_value: adjustValue,
			set_value: setValue,
		});
	}

	function handleModeChange() {
		saveSettings();
	}

	function handleUserChange() {
		saveSettings();
	}
</script>

<div class="p-3">
	<h3 class="mb-3 text-sm font-semibold text-neutral-100">User Volume/Mute Control</h3>
	<p class="mb-3 text-xs text-neutral-400">
		Control another user's volume or mute state. The user must be in a voice channel with you.
	</p>

	<div class="mb-3">
		<div class="mb-1 flex items-center justify-between">
			<label for="userPicker" class="block text-xs text-neutral-200">User in Current Voice Channel</label>
			<button
				type="button"
				on:click={() => sendToPlugin({ action: "get_users" })}
				class="text-xs text-neutral-400 hover:text-neutral-200"
			>
				Refresh
			</button>
		</div>
		<select
			id="userPicker"
			bind:value={userId}
			on:change={handleUserChange}
			class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 focus:border-neutral-600 focus:outline-none"
		>
			<option value="">Select a user...</option>
			{#each users as user}
				<option value={user.user_id}>
					{user.nick ? `${user.nick} (${user.name})` : user.name}
					{user.mute ? " [Muted]" : ""}
					{` ${user.volume}%`}
				</option>
			{/each}
		</select>
		{#if users.length === 0}
			<p class="mt-1 text-xs text-neutral-500">No voice participants detected in the current Discord voice channel.</p>
		{/if}
	</div>

	<div class="mb-3">
		<label for="userId" class="mb-1 block text-xs text-neutral-200">User ID</label>
		<input
			id="userId"
			type="text"
			bind:value={userId}
			on:change={saveSettings}
			placeholder="Discord user ID (snowflake)"
			class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 placeholder-neutral-500 focus:border-neutral-600 focus:outline-none"
		/>
	</div>

	<div class="mb-3">
		<label for="mode" class="mb-1 block text-xs text-neutral-200">Mode</label>
		<select
			id="mode"
			bind:value={mode}
			on:change={handleModeChange}
			class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 focus:border-neutral-600 focus:outline-none"
		>
			<option value="mute">Mute</option>
			<option value="adjust">Adjust Volume</option>
			<option value="set">Set Volume</option>
		</select>
	</div>

	{#if mode === "mute"}
		<div class="mb-3">
			<label for="muteType" class="mb-1 block text-xs text-neutral-200">Mute Type</label>
			<select
				id="muteType"
				bind:value={muteType}
				on:change={saveSettings}
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 focus:border-neutral-600 focus:outline-none"
			>
				<option value="toggle">Toggle</option>
				<option value="mute">Mute</option>
				<option value="unmute">Unmute</option>
			</select>
		</div>
	{:else if mode === "adjust"}
		<div class="mb-3">
			<label for="adjustValue" class="mb-1 block text-xs text-neutral-200">Adjust Value (-25 to +24)</label>
			<input
				id="adjustValue"
				type="number"
				bind:value={adjustValue}
				on:change={saveSettings}
				min="-25"
				max="24"
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 focus:border-neutral-600 focus:outline-none"
			/>
		</div>
	{:else if mode === "set"}
		<div class="mb-3">
			<label for="setValue" class="mb-1 block text-xs text-neutral-200">Volume (0 to 200)</label>
			<input
				id="setValue"
				type="number"
				bind:value={setValue}
				on:change={saveSettings}
				min="0"
				max="200"
				class="w-full rounded-lg border border-neutral-600 bg-neutral-700 px-2 py-1 text-xs text-neutral-100 focus:border-neutral-600 focus:outline-none"
			/>
		</div>
	{/if}

	{#if userId}
		<p class="text-xs text-neutral-500">Selected user ID: {userId}</p>
	{/if}
</div>
