use crate::client::schedule_reconnect;
use crate::client::discord_client;
use crate::current_settings;
use crate::{
	PlaySoundboardSoundAction,
	actions::{SoundInfo, SoundsResponse, UserVolumeControlButtonAction, UserVolumeControlDialAction},
};

use discord_ipc_rust::models::receive::{
	ReceivedItem, commands::ReturnedCommand, events::ReturnedEvent,
};
use discord_ipc_rust::models::soundboard::SoundboardSound;
use discord_ipc_rust::models::send::commands::SentCommand;
use discord_ipc_rust::models::send::events::SubscribeableEvent;
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
	let mut users: Vec<_> = voice_participants().read().await.values().cloned().collect();
	users.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
	users
}

pub async fn current_voice_participant(user_id: &str) -> Option<VoiceParticipant> {
	voice_participants().read().await.get(user_id).cloned()
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
			return;
		}
		let previous = subscribed.clone();
		*subscribed = normalized.clone();
		previous
	};

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

async fn upsert_voice_participant(voice: discord_ipc_rust::models::receive::events::VoiceStateData) {
	let Some(user) = voice.user else {
		return;
	};

	voice_participants().write().await.insert(
		user.id.clone(),
		VoiceParticipant {
			user_id: user.id,
			name: user.username,
			nick: if voice.nick.is_empty() {
				None
			} else {
				Some(voice.nick)
			},
			volume: voice.volume.round().clamp(0.0, 200.0) as i32,
			mute: voice.mute,
		},
	);
	broadcast_voice_participants().await;
}

async fn remove_voice_participant(voice: discord_ipc_rust::models::receive::events::VoiceStateData) {
	let Some(user) = voice.user else {
		return;
	};

	voice_participants().write().await.remove(&user.id);
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
				if error.code == 4002 && error.message.contains("REQUEST_SOUNDBOARD_SOUNDS") {
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
				sync_voice_channel_subscriptions(Some(data.channel_id)).await;
			}
			ReturnedEvent::VoiceStateCreate(voice) | ReturnedEvent::VoiceStateUpdate(voice) => {
				upsert_voice_participant(voice).await;
			}
			ReturnedEvent::VoiceStateDelete(voice) => {
				remove_voice_participant(voice).await;
			}
			ReturnedEvent::SoundboardSounds(sounds) => {
				if let Some(first) = sounds.first() {
					let guild_id = first.guild_id.clone();
					log::debug!("Received {} soundboard sounds for guild {}", sounds.len(), guild_id);
					let sound_infos: Vec<SoundInfo> = sounds
						.iter()
						.map(|sound| SoundInfo {
							sound_id: sound.sound_id.clone(),
							name: sound.name.clone(),
							emoji_name: sound.emoji_name.clone(),
						})
						.collect();
					let mut sounds_map = soundboard_sounds().write().await;
					sounds_map.insert(guild_id.clone(), sounds);
					log::debug!("Stored soundboard sounds for guild {}", guild_id);
					let response = SoundsResponse {
						action: "sounds_result".to_string(),
						sounds: sound_infos,
						error: None,
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
			}
			_ => {}
		},
		ReceivedItem::Command(command) => {
			match *command {
				ReturnedCommand::GetVoiceSettings(voice) => {
					apply_voice_state(voice.mute, voice.deaf).await;
				}
				ReturnedCommand::GetSelectedVoiceChannel(channel) => {
					sync_voice_channel_subscriptions(channel.map(|channel| channel.id)).await;
				}
				_ => {}
			}
		}
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
	update_action_state(crate::actions::ToggleMuteAction::UUID, mute).await;
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
