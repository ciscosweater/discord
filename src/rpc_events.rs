use crate::actions::{
	ChannelsResponse, GuildInfo, SoundInfo, SoundsResponse, UserVolumeControlButtonAction,
	UserVolumeControlDialAction, VoiceChannelInfo,
};
use crate::client::schedule_reconnect;
use crate::client::{clear_authenticated_user, discord_client};
use crate::current_settings;

use discord_ipc_rust::models::receive::{
	CommandResponse, ReceivedItem, commands::ReturnedCommand, events::ReturnedEvent,
};
use discord_ipc_rust::models::send::commands::{GetChannelArgs, SentCommand};
use discord_ipc_rust::models::send::events::SubscribeableEvent;
use discord_ipc_rust::models::shared::{Channel, ChannelType};
use discord_ipc_rust::models::soundboard::SoundboardSound;
use openaction::{Action as _, ActionUuid, get_instance, set_global_settings, visible_instances};

use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::RwLock;

// Store soundboard sounds per guild
pub fn soundboard_sounds() -> &'static RwLock<HashMap<String, Vec<SoundboardSound>>> {
	static SOUNDS: OnceLock<RwLock<HashMap<String, Vec<SoundboardSound>>>> = OnceLock::new();
	SOUNDS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub async fn clear_soundboard_sounds_for_guild(guild_id: &str) {
	soundboard_sounds().write().await.remove(guild_id);
}

#[derive(Clone, Debug)]
pub struct PendingSoundboardRequest {
	pub instance_id: String,
	pub guild_id: String,
}

pub fn pending_soundboard_requests() -> &'static RwLock<HashMap<String, PendingSoundboardRequest>> {
	static REQUESTS: OnceLock<RwLock<HashMap<String, PendingSoundboardRequest>>> = OnceLock::new();
	REQUESTS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub async fn register_pending_soundboard_request(nonce: &str, instance_id: &str, guild_id: &str) {
	pending_soundboard_requests().write().await.insert(
		nonce.to_string(),
		PendingSoundboardRequest {
			instance_id: instance_id.to_string(),
			guild_id: guild_id.to_string(),
		},
	);
}

pub async fn unregister_pending_soundboard_request(
	nonce: &str,
) -> Option<PendingSoundboardRequest> {
	pending_soundboard_requests().write().await.remove(nonce)
}

pub async fn clear_pending_soundboard_requests() {
	pending_soundboard_requests().write().await.clear();
}

#[derive(Clone, Debug)]
pub struct PendingVoiceChannelRequest {
	pub instance_id: String,
	pub guild_id: String,
}

pub fn pending_voice_channel_requests()
-> &'static RwLock<HashMap<String, PendingVoiceChannelRequest>> {
	static REQUESTS: OnceLock<RwLock<HashMap<String, PendingVoiceChannelRequest>>> =
		OnceLock::new();
	REQUESTS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub async fn register_pending_voice_channel_request(
	nonce: &str,
	instance_id: &str,
	guild_id: &str,
) {
	pending_voice_channel_requests().write().await.insert(
		nonce.to_string(),
		PendingVoiceChannelRequest {
			instance_id: instance_id.to_string(),
			guild_id: guild_id.to_string(),
		},
	);
}

pub async fn unregister_pending_voice_channel_request(
	nonce: &str,
) -> Option<PendingVoiceChannelRequest> {
	pending_voice_channel_requests().write().await.remove(nonce)
}

pub async fn clear_pending_voice_channel_requests() {
	pending_voice_channel_requests().write().await.clear();
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

pub fn voice_channels() -> &'static RwLock<HashMap<String, Vec<VoiceChannelInfo>>> {
	static CHANNELS: OnceLock<RwLock<HashMap<String, Vec<VoiceChannelInfo>>>> = OnceLock::new();
	CHANNELS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn voice_channel_request_error() -> &'static RwLock<Option<String>> {
	static REQUEST_ERROR: OnceLock<RwLock<Option<String>>> = OnceLock::new();
	REQUEST_ERROR.get_or_init(|| RwLock::new(None))
}

async fn broadcast_soundboard_guilds(guilds: Vec<GuildInfo>, error: Option<String>) {
	*available_soundboard_guilds().write().await = guilds.clone();
	*guild_request_error().write().await = error.clone();
}

fn guild_icon_hash_from_url(icon_url: Option<&str>) -> Option<String> {
	let icon_url = icon_url?.trim();
	if icon_url.is_empty() {
		return None;
	}

	let path = icon_url.split('?').next().unwrap_or(icon_url);
	let filename = path.rsplit('/').next()?;
	let hash = filename.split('.').next()?.trim();
	if hash.is_empty() {
		return None;
	}

	Some(hash.to_string())
}

fn filter_voice_channels(
	requested_guild_id: Option<&str>,
	channels: Vec<Channel>,
) -> (Option<String>, Vec<VoiceChannelInfo>) {
	let cache_guild_id = requested_guild_id
		.map(str::to_string)
		.or_else(|| channels.first().and_then(|first| first.guild_id.clone()));
	let filtered_channels = channels
		.into_iter()
		.filter(|channel| {
			matches!(
				channel.channel_type,
				ChannelType::GuildVoice | ChannelType::GuildStageVoice
			)
		})
		.map(|channel| VoiceChannelInfo {
			channel_id: channel.id.clone(),
			name: channel
				.name
				.as_deref()
				.map(str::trim)
				.filter(|name| !name.is_empty())
				.map(str::to_string)
				.unwrap_or(channel.id),
		})
		.collect();
	(cache_guild_id, filtered_channels)
}

async fn cache_voice_channels_response(
	requested_guild_id: Option<String>,
	channels: Vec<Channel>,
	error: Option<String>,
) -> Vec<VoiceChannelInfo> {
	let (cache_guild_id, filtered_channels) =
		filter_voice_channels(requested_guild_id.as_deref(), channels);
	if let Some(guild_id) = cache_guild_id {
		voice_channels()
			.write()
			.await
			.insert(guild_id, filtered_channels.clone());
	}

	*voice_channel_request_error().write().await = None;
	if let Some(error) = error {
		*voice_channel_request_error().write().await = Some(error);
	}

	filtered_channels
}

async fn send_voice_channels_response_to_instance(
	instance_id: &str,
	guild_id: &str,
	channels: &[VoiceChannelInfo],
	error: Option<String>,
) {
	let Some(instance) = get_instance(instance_id.to_string()).await else {
		log::debug!(
			"Skipping direct voice channel response for missing instance {}",
			instance_id
		);
		return;
	};

	let payload = match serde_json::to_string(&ChannelsResponse {
		action: "channels_result".to_string(),
		guild_id: guild_id.to_string(),
		channels: channels.to_vec(),
		error,
	}) {
		Ok(payload) => payload,
		Err(error) => {
			log::error!(
				"Failed to serialize direct voice channel response for instance {}: {}",
				instance_id,
				error
			);
			return;
		}
	};

	if let Err(error) = instance.send_to_property_inspector(&payload).await {
		log::error!(
			"Failed to send direct voice channel response to instance {}: {}",
			instance_id,
			error
		);
	}
}

fn filter_soundboard_sounds(
	requested_guild_id: Option<&str>,
	sounds: Vec<SoundboardSound>,
) -> (Option<String>, Vec<SoundboardSound>) {
	let cache_guild_id = requested_guild_id.map(str::to_string).or_else(|| {
		sounds.first().map(|first| {
			if first.guild_id == "0" {
				"DEFAULT".to_string()
			} else {
				first.guild_id.clone()
			}
		})
	});
	let filtered_sounds = match requested_guild_id {
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
	(cache_guild_id, filtered_sounds)
}

async fn cache_soundboard_response(
	requested_guild_id: Option<String>,
	sounds: Vec<SoundboardSound>,
	error: Option<String>,
) -> Vec<SoundboardSound> {
	let (cache_guild_id, filtered_sounds) =
		filter_soundboard_sounds(requested_guild_id.as_deref(), sounds);
	if let Some(guild_id) = cache_guild_id {
		soundboard_sounds()
			.write()
			.await
			.insert(guild_id.clone(), filtered_sounds.clone());
	}

	*soundboard_request_error().write().await = None;
	if let Some(error) = error {
		*soundboard_request_error().write().await = Some(error);
	}

	filtered_sounds
}

async fn send_soundboard_response_to_instance(
	instance_id: &str,
	guild_id: &str,
	sounds: &[SoundboardSound],
	error: Option<String>,
) {
	let Some(instance) = get_instance(instance_id.to_string()).await else {
		log::debug!(
			"Skipping direct soundboard response for missing instance {}",
			instance_id
		);
		return;
	};

	let payload = match serde_json::to_string(&SoundsResponse {
		action: "sounds_result".to_string(),
		guild_id: guild_id.to_string(),
		sounds: sounds
			.iter()
			.map(|sound| SoundInfo {
				sound_id: sound.sound_id.clone(),
				name: sound.name.clone(),
				emoji_name: sound.emoji_name.clone(),
			})
			.collect(),
		error,
	}) {
		Ok(payload) => payload,
		Err(error) => {
			log::error!(
				"Failed to serialize direct soundboard response for instance {}: {}",
				instance_id,
				error
			);
			return;
		}
	};

	if let Err(error) = instance.send_to_property_inspector(&payload).await {
		log::error!(
			"Failed to send direct soundboard response to instance {}: {}",
			instance_id,
			error
		);
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
	#[serde(skip_serializing_if = "Option::is_none")]
	pub avatar_hash: Option<String>,
	#[serde(skip_serializing)]
	pub local_mute: bool,
	#[serde(skip_serializing)]
	pub server_mute: bool,
	#[serde(skip_serializing)]
	pub self_mute: bool,
	#[serde(skip_serializing)]
	pub server_deaf: bool,
	#[serde(skip_serializing)]
	pub self_deaf: bool,
	#[serde(skip_serializing)]
	pub suppress: bool,
}

impl VoiceParticipant {
	pub fn compute_effective_mute(
		local_mute: bool,
		server_mute: bool,
		self_mute: bool,
		server_deaf: bool,
		self_deaf: bool,
		suppress: bool,
	) -> bool {
		local_mute || server_mute || self_mute || server_deaf || self_deaf || suppress
	}

	fn refresh_effective_mute(&mut self) {
		self.mute = Self::compute_effective_mute(
			self.local_mute,
			self.server_mute,
			self.self_mute,
			self.server_deaf,
			self.self_deaf,
			self.suppress,
		);
	}
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

pub async fn apply_local_user_voice_update(user_id: &str, volume: Option<i32>, mute: Option<bool>) {
	let mut participants = voice_participants().write().await;
	let Some(participant) = participants.get_mut(user_id) else {
		return;
	};

	if let Some(volume) = volume {
		participant.volume = volume.clamp(0, 200);
	}
	if let Some(mute) = mute {
		participant.local_mute = mute;
		participant.refresh_effective_mute();
	}
	drop(participants);

	broadcast_voice_participants().await;
}

pub async fn current_subscribed_voice_channel() -> Option<String> {
	subscribed_voice_channel().read().await.clone()
}

pub async fn clear_voice_participants() {
	voice_participants().write().await.clear();
	broadcast_voice_participants().await;
}

async fn broadcast_voice_participants() {
	let users = current_voice_participants().await;
	let payload = match serde_json::to_string(&VoiceParticipantsResponse {
		action: "users_result".to_string(),
		users,
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

	crate::actions::refresh_user_volume_instances().await;
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
		mute: VoiceParticipant::compute_effective_mute(
			voice.mute,
			voice.state.mute,
			voice.state.self_mute,
			voice.state.deaf,
			voice.state.self_deaf,
			voice.state.suppress,
		),
		avatar_hash: user.avatar.clone(),
		local_mute: voice.mute,
		server_mute: voice.state.mute,
		self_mute: voice.state.self_mute,
		server_deaf: voice.state.deaf,
		self_deaf: voice.state.self_deaf,
		suppress: voice.state.suppress,
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
		log::debug!(
			"Skipping voice channel snapshot request because Discord client is unavailable"
		);
		return;
	};

	log::debug!("Requesting GetSelectedVoiceChannel snapshot");
	if let Err(error) = client
		.emit_command(&SentCommand::GetSelectedVoiceChannel)
		.await
	{
		log::warn!(
			"Failed to request current voice channel snapshot: {}",
			error
		);
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
		log::warn!(
			"Failed to request channel details for {}: {}",
			channel_id,
			error
		);
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

	voice_participants()
		.write()
		.await
		.insert(participant.user_id.clone(), participant.clone());
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
					clear_pending_soundboard_requests().await;
				}
				if error.code == 4006 {
					let mut current = current_settings().write().await;
					current.access_token.clear();
					if let Err(e) = set_global_settings(&*current).await {
						log::error!("Failed to clear access token in settings: {}", e);
					}
					clear_pending_soundboard_requests().await;
					clear_pending_voice_channel_requests().await;
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
				let guild_id = sounds.first().map(|first| {
					if first.guild_id == "0" {
						"DEFAULT".to_string()
					} else {
						first.guild_id.clone()
					}
				});
				let _ = cache_soundboard_response(guild_id, sounds, None).await;
			}
			_ => {}
		},
		ReceivedItem::Command(command_response) => {
			let CommandResponse { nonce, command } = *command_response;
			match command {
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
						channel
							.voice_states
							.as_ref()
							.map(|states| states.len())
							.unwrap_or(0)
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
							icon_hash: guild_icon_hash_from_url(guild.icon_url.as_deref()),
						})
						.collect();
					guilds.sort_by(|left, right| {
						left.name.to_lowercase().cmp(&right.name.to_lowercase())
					});
					guilds.insert(
						0,
						GuildInfo {
							guild_id: "DEFAULT".to_string(),
							name: "Discord Default Sounds".to_string(),
							icon_hash: None,
						},
					);
					broadcast_soundboard_guilds(guilds, None).await;
				}
				ReturnedCommand::GetSoundboardSounds(sounds) => {
					let pending_request = match nonce.as_deref() {
						Some(nonce) => unregister_pending_soundboard_request(nonce).await,
						None => None,
					};
					let requested_guild_id = pending_request
						.as_ref()
						.map(|request| request.guild_id.clone());
					let filtered_sounds =
						cache_soundboard_response(requested_guild_id, sounds, None).await;
					if let Some(request) = pending_request {
						send_soundboard_response_to_instance(
							&request.instance_id,
							&request.guild_id,
							&filtered_sounds,
							None,
						)
						.await;
					}
				}
				ReturnedCommand::GetChannels(channels) => {
					let pending_request = match nonce.as_deref() {
						Some(nonce) => unregister_pending_voice_channel_request(nonce).await,
						None => None,
					};
					let requested_guild_id = pending_request
						.as_ref()
						.map(|request| request.guild_id.clone());
					let filtered_channels =
						cache_voice_channels_response(requested_guild_id, channels, None).await;
					if let Some(request) = pending_request {
						send_voice_channels_response_to_instance(
							&request.instance_id,
							&request.guild_id,
							&filtered_channels,
							None,
						)
						.await;
					}
				}
				_ => {}
			}
		}
		ReceivedItem::SocketClosed => {
			log::warn!("Discord closed; attempting to reconnect");
			clear_voice_participants().await;
			clear_authenticated_user().await;
			clear_pending_soundboard_requests().await;
			clear_pending_voice_channel_requests().await;
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

#[cfg(test)]
mod tests {
	use super::{VoiceParticipant, filter_voice_channels};
	use discord_ipc_rust::models::shared::{Channel, ChannelType};

	#[test]
	fn effective_mute_is_false_when_all_flags_are_false() {
		assert!(!VoiceParticipant::compute_effective_mute(
			false, false, false, false, false, false
		));
	}

	#[test]
	fn effective_mute_is_true_when_any_flag_is_true() {
		for flags in [
			(true, false, false, false, false, false),
			(false, true, false, false, false, false),
			(false, false, true, false, false, false),
			(false, false, false, true, false, false),
			(false, false, false, false, true, false),
			(false, false, false, false, false, true),
		] {
			assert!(VoiceParticipant::compute_effective_mute(
				flags.0, flags.1, flags.2, flags.3, flags.4, flags.5
			));
		}
	}

	#[test]
	fn filter_voice_channels_keeps_only_voice_and_stage() {
		let channels = vec![
			Channel {
				id: "text".to_string(),
				channel_type: ChannelType::GuildText,
				guild_id: Some("guild".to_string()),
				position: None,
				name: Some("general".to_string()),
				topic: None,
				nsfw: None,
				last_message_id: None,
				bitrate: None,
				user_limit: None,
				rate_limit_per_user: None,
				voice_states: None,
			},
			Channel {
				id: "voice".to_string(),
				channel_type: ChannelType::GuildVoice,
				guild_id: Some("guild".to_string()),
				position: None,
				name: Some("Standup".to_string()),
				topic: None,
				nsfw: None,
				last_message_id: None,
				bitrate: None,
				user_limit: None,
				rate_limit_per_user: None,
				voice_states: None,
			},
			Channel {
				id: "stage".to_string(),
				channel_type: ChannelType::GuildStageVoice,
				guild_id: Some("guild".to_string()),
				position: None,
				name: Some("Town Hall".to_string()),
				topic: None,
				nsfw: None,
				last_message_id: None,
				bitrate: None,
				user_limit: None,
				rate_limit_per_user: None,
				voice_states: None,
			},
		];

		let (guild_id, filtered) = filter_voice_channels(Some("guild"), channels);
		assert_eq!(guild_id.as_deref(), Some("guild"));
		assert_eq!(filtered.len(), 2);
		assert_eq!(filtered[0].channel_id, "voice");
		assert_eq!(filtered[1].channel_id, "stage");
	}
}
