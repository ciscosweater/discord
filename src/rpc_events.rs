use crate::client::discord_client;
use crate::client::schedule_reconnect;
use crate::current_settings;
use crate::{
	PlaySoundboardSoundAction,
	actions::{
		GuildInfo, GuildsResponse, SoundInfo, SoundsResponse, UserVolumeControlButtonAction,
		UserVolumeControlDialAction,
	},
};

use discord_ipc_rust::models::receive::{
	ReceivedItem, commands::ReturnedCommand, events::ReturnedEvent,
};
use discord_ipc_rust::models::send::commands::{GetChannelArgs, SentCommand};
use discord_ipc_rust::models::send::events::SubscribeableEvent;
use discord_ipc_rust::models::shared::Channel;
use discord_ipc_rust::models::soundboard::SoundboardSound;
use openaction::{Action as _, ActionUuid, set_global_settings, visible_instances};

use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::RwLock;

// Store soundboard sounds per guild
pub fn soundboard_sounds() -> &'static RwLock<HashMap<String, Vec<SoundboardSound>>> {
	static SOUNDS: OnceLock<RwLock<HashMap<String, Vec<SoundboardSound>>>> = OnceLock::new();
	SOUNDS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn pending_soundboard_guild() -> &'static RwLock<Option<String>> {
	static PENDING_GUILD: OnceLock<RwLock<Option<String>>> = OnceLock::new();
	PENDING_GUILD.get_or_init(|| RwLock::new(None))
}

pub async fn set_pending_soundboard_guild(guild_id: Option<String>) {
	*pending_soundboard_guild().write().await = guild_id;
}

pub fn soundboard_request_error() -> &'static RwLock<Option<String>> {
	static REQUEST_ERROR: OnceLock<RwLock<Option<String>>> = OnceLock::new();
	REQUEST_ERROR.get_or_init(|| RwLock::new(None))
}

pub fn available_soundboard_guilds() -> &'static RwLock<Vec<GuildInfo>> {
	static GUILDS: OnceLock<RwLock<Vec<GuildInfo>>> = OnceLock::new();
	GUILDS.get_or_init(|| RwLock::new(Vec::new()))
}

pub fn guild_request_error() -> &'static RwLock<Option<String>> {
	static REQUEST_ERROR: OnceLock<RwLock<Option<String>>> = OnceLock::new();
	REQUEST_ERROR.get_or_init(|| RwLock::new(None))
}

async fn broadcast_soundboard_guilds(guilds: Vec<GuildInfo>, error: Option<String>) {
	*available_soundboard_guilds().write().await = guilds.clone();
	*guild_request_error().write().await = error.clone();

	let payload = match serde_json::to_string(&GuildsResponse {
		action: "guilds_result".to_string(),
		guilds,
		error,
	}) {
		Ok(payload) => payload,
		Err(error) => {
			log::error!("Failed to serialize guild list response: {}", error);
			return;
		}
	};

	for instance in visible_instances(PlaySoundboardSoundAction::UUID).await {
		if let Err(error) = instance.send_to_property_inspector(&payload).await {
			log::error!("Failed to forward guild list to PI: {}", error);
		}
	}
}

async fn broadcast_soundboard_response(
	guild_id: Option<String>,
	sounds: Vec<SoundboardSound>,
	error: Option<String>,
) {
	let requested_guild_id = pending_soundboard_guild().read().await.clone();
	let cache_guild_id = requested_guild_id.or(guild_id);
	let filtered_sounds = match cache_guild_id.as_deref() {
		Some("DEFAULT") => sounds
			.into_iter()
			.filter(|sound| sound.guild_id == "0")
			.collect(),
		Some(requested_guild_id) => sounds
			.into_iter()
			.filter(|sound| sound.guild_id == requested_guild_id)
			.collect(),
		None => sounds,
	};

	if let Some(guild_id) = cache_guild_id {
		soundboard_sounds()
			.write()
			.await
			.insert(guild_id.clone(), filtered_sounds.clone());
	}

	let response = SoundsResponse {
		action: "sounds_result".to_string(),
		sounds: filtered_sounds
			.iter()
			.map(|sound| SoundInfo {
				sound_id: sound.sound_id.clone(),
				name: sound.name.clone(),
				emoji_name: sound.emoji_name.clone(),
			})
			.collect(),
		error,
	};

	*soundboard_request_error().write().await = None;
	set_pending_soundboard_guild(None).await;

	let payload = match serde_json::to_string(&response) {
		Ok(payload) => payload,
		Err(error) => {
			log::error!("Failed to serialize soundboard sounds response: {}", error);
			return;
		}
	};

	for instance in visible_instances(PlaySoundboardSoundAction::UUID).await {
		if let Err(error) = instance.send_to_property_inspector(&payload).await {
			log::error!("Failed to forward soundboard sounds to PI: {}", error);
		}
	}
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct VoiceParticipant {
	pub user_id: String,
	pub name: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub nick: Option<String>,
	pub volume: i32,
	pub mute: bool,
}

#[derive(Debug, serde::Serialize)]
struct VoiceParticipantsResponse {
	action: String,
	users: Vec<VoiceParticipant>,
}

pub fn voice_participants() -> &'static RwLock<HashMap<String, VoiceParticipant>> {
	static PARTICIPANTS: OnceLock<RwLock<HashMap<String, VoiceParticipant>>> = OnceLock::new();
	PARTICIPANTS.get_or_init(|| RwLock::new(HashMap::new()))
}

fn subscribed_voice_channel() -> &'static RwLock<Option<String>> {
	static CHANNEL: OnceLock<RwLock<Option<String>>> = OnceLock::new();
	CHANNEL.get_or_init(|| RwLock::new(None))
}

pub async fn current_voice_participants() -> Vec<VoiceParticipant> {
	let mut users: Vec<_> = voice_participants()
		.read()
		.await
		.values()
		.cloned()
		.collect();
	users.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
	users
}

pub async fn current_voice_participant(user_id: &str) -> Option<VoiceParticipant> {
	voice_participants().read().await.get(user_id).cloned()
}

pub async fn current_subscribed_voice_channel() -> Option<String> {
	subscribed_voice_channel().read().await.clone()
}

pub async fn clear_voice_participants() {
	voice_participants().write().await.clear();
	broadcast_voice_participants().await;
}

async fn broadcast_voice_participants() {
	let payload = match serde_json::to_string(&VoiceParticipantsResponse {
		action: "users_result".to_string(),
		users: current_voice_participants().await,
	}) {
		Ok(payload) => payload,
		Err(error) => {
			log::error!("Failed to serialize voice participants response: {}", error);
			return;
		}
	};

	for instance in visible_instances(UserVolumeControlButtonAction::UUID).await {
		if let Err(error) = instance.send_to_property_inspector(&payload).await {
			log::error!("Failed to forward users to button PI: {}", error);
		}
	}

	for instance in visible_instances(UserVolumeControlDialAction::UUID).await {
		if let Err(error) = instance.send_to_property_inspector(&payload).await {
			log::error!("Failed to forward users to dial PI: {}", error);
		}
	}
}

async fn sync_voice_channel_subscriptions(channel_id: Option<String>) {
	let normalized = channel_id.and_then(|channel_id| {
		let trimmed = channel_id.trim();
		if trimmed.is_empty() {
			None
		} else {
			Some(trimmed.to_string())
		}
	});

	let previous = {
		let mut subscribed = subscribed_voice_channel().write().await;
		if *subscribed == normalized {
			log::debug!(
				"Voice channel subscription unchanged: {}",
				normalized.as_deref().unwrap_or("<none>")
			);
			return;
		}
		let previous = subscribed.clone();
		*subscribed = normalized.clone();
		previous
	};

	log::info!(
		"Switching voice channel subscription from {} to {}",
		previous.as_deref().unwrap_or("<none>"),
		normalized.as_deref().unwrap_or("<none>")
	);
	clear_voice_participants().await;

	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		return;
	};

	if let Some(previous) = previous {
		for event in [
			SubscribeableEvent::VoiceStateCreate {
				channel_id: previous.clone(),
			},
			SubscribeableEvent::VoiceStateUpdate {
				channel_id: previous.clone(),
			},
			SubscribeableEvent::VoiceStateDelete {
				channel_id: previous.clone(),
			},
		] {
			if let Err(error) = client.emit_command(&SentCommand::Unsubscribe(event)).await {
				log::warn!("Failed to unsubscribe voice state event: {}", error);
			}
		}
	}

	if let Some(channel_id) = normalized {
		for event in [
			SubscribeableEvent::VoiceStateCreate {
				channel_id: channel_id.clone(),
			},
			SubscribeableEvent::VoiceStateUpdate {
				channel_id: channel_id.clone(),
			},
			SubscribeableEvent::VoiceStateDelete {
				channel_id: channel_id.clone(),
			},
		] {
			if let Err(error) = client.emit_command(&SentCommand::Subscribe(event)).await {
				log::warn!("Failed to subscribe voice state event: {}", error);
			}
		}
	}
}

fn voice_participant_from_state(
	voice: &discord_ipc_rust::models::receive::events::VoiceStateData,
) -> Option<VoiceParticipant> {
	let user = voice.user.as_ref()?;

	Some(VoiceParticipant {
		user_id: user.id.clone(),
		name: user.username.clone(),
		nick: if voice.nick.is_empty() {
			None
		} else {
			Some(voice.nick.clone())
		},
		volume: voice.volume.round().clamp(0.0, 200.0) as i32,
		mute: voice.mute,
	})
}

async fn replace_voice_participants(
	channel_id: &str,
	voices: &[discord_ipc_rust::models::receive::events::VoiceStateData],
) {
	let participants: HashMap<String, VoiceParticipant> = voices
		.iter()
		.filter_map(voice_participant_from_state)
		.map(|participant| (participant.user_id.clone(), participant))
		.collect();
	let count = voices.len();
	let mapped_count = participants.len();

	*voice_participants().write().await = participants;
	log::info!(
		"Loaded {} voice participants from snapshot for channel {} ({} voice states received)",
		mapped_count,
		channel_id,
		count
	);
	broadcast_voice_participants().await;
}

async fn request_selected_voice_channel_snapshot() {
	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		log::debug!("Skipping voice channel snapshot request because Discord client is unavailable");
		return;
	};

	log::debug!("Requesting GetSelectedVoiceChannel snapshot");
	if let Err(error) = client.emit_command(&SentCommand::GetSelectedVoiceChannel).await {
		log::warn!("Failed to request current voice channel snapshot: {}", error);
	}
}

async fn request_channel_details(channel_id: &str) {
	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		log::debug!(
			"Skipping GetChannel fallback for {} because Discord client is unavailable",
			channel_id
		);
		return;
	};

	log::debug!("Requesting GetChannel fallback for {}", channel_id);
	if let Err(error) = client
		.emit_command(&SentCommand::GetChannel(GetChannelArgs {
			channel_id: channel_id.to_string(),
		}))
		.await
	{
		log::warn!("Failed to request channel details for {}: {}", channel_id, error);
	}
}

async fn handle_selected_voice_channel(channel: Option<Channel>) {
	let channel_id = channel.as_ref().map(|channel| channel.id.clone());
	sync_voice_channel_subscriptions(channel_id).await;

	match channel {
		Some(channel) => {
			let voice_states_present = channel.voice_states.is_some();
			let snapshot = channel.voice_states.unwrap_or_default();
			log::info!(
				"Received selected voice channel {} snapshot: voice_states_present={}, participants={}",
				channel.id,
				voice_states_present,
				snapshot.len()
			);
			replace_voice_participants(&channel.id, &snapshot).await;
			if snapshot.is_empty() {
				log::warn!(
					"Selected voice channel {} returned empty voice_states snapshot; requesting GetChannel fallback",
					channel.id
				);
				request_channel_details(&channel.id).await;
			}
		}
		None => {
			log::info!("Discord reports no selected voice channel");
		}
	}
}

async fn upsert_voice_participant(
	voice: discord_ipc_rust::models::receive::events::VoiceStateData,
) {
	let Some(participant) = voice_participant_from_state(&voice) else {
		log::debug!("Ignoring voice state update without user payload");
		return;
	};

	voice_participants().write().await.insert(
		participant.user_id.clone(),
		participant.clone(),
	);
	log::debug!(
		"Updated voice participant {} (mute={}, volume={})",
		participant.user_id,
		participant.mute,
		participant.volume
	);
	broadcast_voice_participants().await;
}

async fn remove_voice_participant(
	voice: discord_ipc_rust::models::receive::events::VoiceStateData,
) {
	let Some(user) = voice.user else {
		log::debug!("Ignoring voice state delete without user payload");
		return;
	};

	voice_participants().write().await.remove(&user.id);
	log::debug!("Removed voice participant {}", user.id);
	broadcast_voice_participants().await;
}

// Central handler for Discord RPC events and command responses we subscribe to (e.g., voice settings).
pub async fn handle_rpc_event(item: ReceivedItem) {
	match item {
		ReceivedItem::Event(event) => match *event {
			ReturnedEvent::Error(error) => {
				log::error!(
					"Discord RPC error: code {:?}, message {:?}",
					error.code,
					error.message
				);
				log::debug!("Full error event data: {:?}", error);
				if error.code == 4002
					&& (error.message.contains("REQUEST_SOUNDBOARD_SOUNDS")
						|| error.message.contains("GET_SOUNDBOARD_SOUNDS"))
				{
					let message = "Quick Select is not supported by this Discord RPC client. Use Manual Entry below.".to_string();
					*soundboard_request_error().write().await = Some(message.clone());
					set_pending_soundboard_guild(None).await;
					let payload = match serde_json::to_string(&SoundsResponse {
						action: "sounds_result".to_string(),
						sounds: vec![],
						error: Some(message),
					}) {
						Ok(payload) => payload,
						Err(error) => {
							log::error!("Failed to serialize soundboard error response: {}", error);
							return;
						}
					};
					for instance in visible_instances(PlaySoundboardSoundAction::UUID).await {
						if let Err(error) = instance.send_to_property_inspector(&payload).await {
							log::error!("Failed to forward soundboard error to PI: {}", error);
						}
					}
				}
				if error.code == 4006 {
					let mut current = current_settings().write().await;
					current.access_token.clear();
					if let Err(e) = set_global_settings(&*current).await {
						log::error!("Failed to clear access token in settings: {}", e);
					}
					schedule_reconnect();
				}
			}
			ReturnedEvent::VoiceSettingsUpdate(voice) => {
				apply_voice_state(voice.mute, voice.deaf).await
			}
			ReturnedEvent::VoiceChannelSelect(data) => {
				let channel_id = data.channel_id.clone();
				log::info!(
					"VOICE_CHANNEL_SELECT received: channel_id={}, guild_id={}",
					channel_id.as_deref().unwrap_or("<none>"),
					data.guild_id.as_deref().unwrap_or("<none>")
				);
				sync_voice_channel_subscriptions(channel_id.clone()).await;
				if channel_id.is_some() {
					request_selected_voice_channel_snapshot().await;
				} else {
					log::info!("Voice channel deselected");
				}
			}
			ReturnedEvent::VoiceStateCreate(voice) | ReturnedEvent::VoiceStateUpdate(voice) => {
				upsert_voice_participant(voice).await;
			}
			ReturnedEvent::VoiceStateDelete(voice) => {
				remove_voice_participant(voice).await;
			}
			ReturnedEvent::SoundboardSounds(sounds) => {
				let guild_id = sounds.first().map(|first| first.guild_id.clone());
				broadcast_soundboard_response(guild_id, sounds, None).await;
			}
			_ => {}
		},
		ReceivedItem::Command(command) => match *command {
			ReturnedCommand::GetVoiceSettings(voice) => {
				apply_voice_state(voice.mute, voice.deaf).await;
			}
			ReturnedCommand::GetSelectedVoiceChannel(channel) => {
				handle_selected_voice_channel(channel).await;
			}
			ReturnedCommand::GetChannel(channel) => {
				let current_channel = current_subscribed_voice_channel().await;
				log::info!(
					"Received GetChannel response: channel_id={}, current_subscription={}, voice_states_present={}, participants={}",
					channel.id,
					current_channel.as_deref().unwrap_or("<none>"),
					channel.voice_states.is_some(),
					channel.voice_states.as_ref().map(|states| states.len()).unwrap_or(0)
				);
				if current_channel.as_deref() == Some(channel.id.as_str()) {
					let snapshot = channel.voice_states.unwrap_or_default();
					replace_voice_participants(&channel.id, &snapshot).await;
				} else {
					log::debug!(
						"Ignoring GetChannel response for {} because current subscription is {}",
						channel.id,
						current_channel.as_deref().unwrap_or("<none>")
					);
				}
			}
			ReturnedCommand::GetGuilds(data) => {
				let mut guilds: Vec<GuildInfo> = data
					.guilds
					.into_iter()
					.map(|guild| GuildInfo {
						guild_id: guild.id,
						name: guild.name,
					})
					.collect();
				guilds.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
				guilds.insert(
					0,
					GuildInfo {
						guild_id: "DEFAULT".to_string(),
						name: "Discord Default Sounds".to_string(),
					},
				);
				broadcast_soundboard_guilds(guilds, None).await;
			}
			ReturnedCommand::GetSoundboardSounds(sounds) => {
				let guild_id = sounds.first().map(|first| first.guild_id.clone());
				broadcast_soundboard_response(guild_id, sounds, None).await;
			}
			_ => {}
		},
		ReceivedItem::SocketClosed => {
			log::warn!("Discord closed; attempting to reconnect");
			clear_voice_participants().await;
			schedule_reconnect();
		}
	}
}

async fn apply_voice_state(mute: Option<bool>, deaf: Option<bool>) {
	let mute = mute.unwrap_or(false);
	let deaf = deaf.unwrap_or(false);
	let effective_mute = mute || deaf;
	update_action_state(crate::actions::ToggleMuteAction::UUID, effective_mute).await;
	update_action_state(crate::actions::ToggleDeafenAction::UUID, deaf).await;
}

async fn update_action_state(action_uuid: ActionUuid, active: bool) {
	let state = if active { 1 } else { 0 };
	for instance in visible_instances(action_uuid).await {
		if let Err(e) = instance.set_state(state).await {
			log::error!("Failed to update state for {}: {}", action_uuid, e);
		}
	}
}
