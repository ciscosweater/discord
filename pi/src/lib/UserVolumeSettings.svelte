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
		error?: string;
	}

	function parseUsersResponse(detail: unknown): UsersResponse | null {
		const candidate =
			detail && typeof detail === "object" && "payload" in detail
				? (detail as { payload: unknown }).payload
				: detail;

		if (typeof candidate === "string") {
			try {
				return JSON.parse(candidate) as UsersResponse;
			} catch {
				return null;
			}
		}
		if (candidate && typeof candidate === "object") {
			return candidate as UsersResponse;
		}
		return null;
	}

	let userId = "";
	let mode = "mute";
	let muteType = "toggle";
	let adjustValue = 0;
	let setValue = 100;
	let users: UserInfo[] = [];
	let loadingUsers = false;
	let usersError: string | null = null;
	let listener: ((event: Event) => void) | null = null;
	let userRequestToken = 0;

	const selectStyle = "background-color: rgb(64 64 64); color: rgb(245 245 245); color-scheme: dark;";
	const optionStyle = "background-color: rgb(38 38 38); color: rgb(245 245 245);";
	const inputStyle = "background-color: rgb(64 64 64); color: rgb(245 245 245); color-scheme: dark;";

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

	$: selectedUserMissing =
		!!userId && users.length > 0 && !users.some((user) => user.user_id === userId);

	function handleUsersResponse(response: UsersResponse) {
		users = response.users ?? [];
		usersError = response.error ?? null;
		loadingUsers = false;
	}

	function refreshUsers() {
		const requestToken = ++userRequestToken;
		loadingUsers = true;
		usersError = null;
		sendToPlugin({ action: "get_users" });
		window.setTimeout(() => {
			if (requestToken === userRequestToken && loadingUsers) {
				loadingUsers = false;
				usersError = "Loading voice participants timed out. Check plugin logs and Discord RPC status.";
			}
		}, 16000);
	}

	onMount(() => {
		listener = (event: Event) => {
			const detail = parseUsersResponse((event as CustomEvent<unknown>).detail);
			if (detail?.action === "users_result") {
				handleUsersResponse(detail);
			}
		};
		eventTarget.addEventListener("sendToPropertyInspector", listener);
		refreshUsers();
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

<style>
	.pi-dark-select {
		background-color: rgb(64 64 64) !important;
		color: rgb(245 245 245) !important;
		color-scheme: dark !important;
		appearance: none;
		-webkit-appearance: none;
		-moz-appearance: none;
		background-image: none !important;
	}

	.pi-dark-select option {
		background-color: rgb(38 38 38) !important;
		color: rgb(245 245 245) !important;
	}
</style>

<div class="rounded-xl border border-neutral-700 bg-neutral-800 p-3 text-neutral-100 shadow-sm">
	<h3 class="mb-3 text-sm font-semibold text-neutral-100">User Volume/Mute Control</h3>
	<p class="mb-3 text-xs text-neutral-400">
		Control another user's volume or mute state. The user must be in a voice channel with you.
	</p>

	<div class="mb-4 rounded-lg border border-neutral-700 bg-neutral-900/60 p-3">
		<div class="mb-1 flex items-center justify-between">
			<label for="userPicker" class="block text-xs text-neutral-200">User in Current Voice Channel</label>
			<button
				type="button"
				on:click={refreshUsers}
				disabled={loadingUsers}
				class="text-xs text-neutral-400 hover:text-neutral-200 disabled:cursor-not-allowed disabled:opacity-50"
			>
				{#if loadingUsers}
					Refreshing...
				{:else}
					Refresh
				{/if}
			</button>
		</div>

		{#if loadingUsers && users.length === 0}
			<div class="flex items-center rounded-lg border border-neutral-700 bg-neutral-800 px-3 py-2 text-xs text-neutral-400">
				<svg class="mr-2 h-4 w-4 animate-spin" viewBox="0 0 24 24">
					<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" fill="none" />
					<path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
				</svg>
				Loading voice participants...
			</div>
		{:else}
			<div class="relative">
				<select
					id="userPicker"
					bind:value={userId}
					on:change={handleUserChange}
					disabled={loadingUsers || users.length === 0}
					class="pi-dark-select w-full rounded-lg border border-neutral-600 px-2 py-1 pr-8 text-xs focus:border-neutral-600 focus:outline-none disabled:cursor-not-allowed disabled:opacity-60"
					style={selectStyle}
				>
					<option value="" style={optionStyle}>Select a user...</option>
					{#each users as user}
						<option value={user.user_id} style={optionStyle}>
							{user.nick ? `${user.nick} (${user.name})` : user.name}
							{user.mute ? " [Muted]" : ""}
							{` ${user.volume}%`}
						</option>
					{/each}
				</select>
				<div class="pointer-events-none absolute inset-y-0 right-0 flex items-center pr-2 text-neutral-300">
					<svg class="h-4 w-4" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
						<path fill-rule="evenodd" d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z" clip-rule="evenodd" />
					</svg>
				</div>
			</div>
		{/if}

		{#if usersError}
			<p class="mt-2 rounded-lg border border-amber-700 bg-amber-950/30 px-3 py-2 text-xs text-amber-300">
				{usersError}
			</p>
		{:else if selectedUserMissing}
			<p class="mt-2 rounded-lg border border-amber-700 bg-amber-950/30 px-3 py-2 text-xs text-amber-300">
				The configured user is not currently present in this voice channel snapshot.
			</p>
		{:else if !loadingUsers && users.length === 0}
			<p class="mt-2 rounded-lg border border-neutral-700 bg-neutral-800 px-3 py-2 text-xs text-neutral-400">
				No voice participants detected in the current Discord voice channel.
			</p>
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
			style={inputStyle}
		/>
	</div>

	<div class="mb-3">
		<label for="mode" class="mb-1 block text-xs text-neutral-200">Mode</label>
		<div class="relative">
			<select
				id="mode"
				bind:value={mode}
				on:change={handleModeChange}
				class="pi-dark-select w-full rounded-lg border border-neutral-600 px-2 py-1 pr-8 text-xs focus:border-neutral-600 focus:outline-none"
				style={selectStyle}
			>
				<option value="mute" style={optionStyle}>Mute</option>
				<option value="adjust" style={optionStyle}>Adjust Volume</option>
				<option value="set" style={optionStyle}>Set Volume</option>
			</select>
			<div class="pointer-events-none absolute inset-y-0 right-0 flex items-center pr-2 text-neutral-300">
				<svg class="h-4 w-4" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
					<path fill-rule="evenodd" d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z" clip-rule="evenodd" />
				</svg>
			</div>
		</div>
	</div>

	{#if mode === "mute"}
		<div class="mb-3">
			<label for="muteType" class="mb-1 block text-xs text-neutral-200">Mute Type</label>
			<div class="relative">
				<select
					id="muteType"
					bind:value={muteType}
					on:change={saveSettings}
					class="pi-dark-select w-full rounded-lg border border-neutral-600 px-2 py-1 pr-8 text-xs focus:border-neutral-600 focus:outline-none"
					style={selectStyle}
				>
					<option value="toggle" style={optionStyle}>Toggle</option>
					<option value="mute" style={optionStyle}>Mute</option>
					<option value="unmute" style={optionStyle}>Unmute</option>
				</select>
				<div class="pointer-events-none absolute inset-y-0 right-0 flex items-center pr-2 text-neutral-300">
					<svg class="h-4 w-4" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
						<path fill-rule="evenodd" d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z" clip-rule="evenodd" />
					</svg>
				</div>
			</div>
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
				style={inputStyle}
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
				style={inputStyle}
			/>
		</div>
	{/if}

	{#if userId}
		<p class="text-xs text-neutral-500">Selected user ID: {userId}</p>
	{/if}
</div>
