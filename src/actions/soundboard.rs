use crate::client::discord_client;
use crate::rpc_events::soundboard_sounds;

use discord_ipc_rust::models::send::commands::RequestSoundboardSoundsArgs;
use std::collections::HashMap;

use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
struct SoundboardPayload {
	action: Option<String>,
	sound_id: Option<String>,
	guild_id: Option<String>,
	sound_name: Option<String>,
}

/// Response sent back to UI when sounds are requested
#[derive(Debug, serde::Serialize)]
pub(crate) struct SoundsResponse {
	pub(crate) action: String,
	pub(crate) sounds: Vec<SoundInfo>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(crate) error: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct SoundInfo {
	pub(crate) sound_id: String,
	pub(crate) name: String,
	pub(crate) emoji_name: Option<String>,
}

/// Request soundboard sounds for a guild via Discord RPC
async fn request_soundboard_sounds_for_guild(guild_id: &str) -> Result<(), String> {
	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		return Err("Discord client not initialized".to_string());
	};

	let args = RequestSoundboardSoundsArgs {
		guild_id: guild_id.to_string(),
	};
	let nonce = Uuid::new_v4().to_string();
	let payload = json!({
		"cmd": "REQUEST_SOUNDBOARD_SOUNDS",
		"args": args,
		"nonce": nonce
	});

	log::debug!("Requesting soundboard sounds for guild {}: {}", guild_id, payload);
	client
		.emit_string(&payload.to_string())
		.await
		.map_err(|e| format!("Failed to request soundboard sounds: {}", e))
}

async fn send_sounds_response(
	instance: &Instance,
	sounds: Vec<SoundInfo>,
	error: Option<String>,
) -> OpenActionResult<()> {
	let response = SoundsResponse {
		action: "sounds_result".to_string(),
		sounds,
		error,
	};
	instance
		.send_to_property_inspector(&serde_json::to_string(&response).unwrap())
		.await
}

pub struct PlaySoundboardSoundAction;
#[async_trait]
impl Action for PlaySoundboardSoundAction {
	const UUID: ActionUuid = "com.elgato.discord.soundboard";
	type Settings = HashMap<String, String>;

	async fn key_up(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		let mut client_lock = discord_client().write().await;
		let Some(client) = client_lock.as_mut() else {
			log::error!("Discord client not initialized");
			instance.show_alert().await?;
			return Ok(());
		};

		let sound_id = settings.get("sound_id").cloned();
		let guild_id = settings.get("guild_id").cloned();

		let Some(sound_id) = sound_id else {
			log::error!("No sound_id provided in settings");
			instance.show_alert().await?;
			return Ok(());
		};

		let Some(guild_id) = guild_id else {
			log::error!("No guild_id provided in settings");
			instance.show_alert().await?;
			return Ok(());
		};

		let args = json!({
			"sound_id": sound_id,
			"guild_id": guild_id
		});
		let nonce = Uuid::new_v4().to_string();
		let payload = json!({
			"cmd": "PLAY_SOUNDBOARD_SOUND",
			"args": args,
			"nonce": nonce
		});

		log::debug!("Soundboard payload being sent: {}", payload);

		match client.emit_string(&payload.to_string()).await {
			Ok(_) => instance.show_ok().await,
			Err(e) => {
				log::error!("Failed to play soundboard sound: {}", e);
				instance.show_alert().await
			}
		}
	}

	async fn send_to_plugin(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
		payload: &serde_json::Value,
	) -> OpenActionResult<()> {
		let payload: SoundboardPayload = match serde_json::from_value(payload.clone()) {
			Ok(p) => p,
			Err(e) => {
				log::error!("Failed to parse soundboard payload: {}", e);
				return Ok(());
			}
		};

		// Handle get_sounds action from UI
		if payload.action.as_deref() == Some("get_sounds") {
			let guild_id = match &payload.guild_id {
				Some(id) => id.clone(),
				None => {
					log::warn!("get_sounds request but no guild_id provided");
					return Ok(());
				}
			};

			// Check if we have cached sounds for this guild
			let sounds_map = soundboard_sounds().read().await;
			if let Some(cached_sounds) = sounds_map.get(&guild_id) {
				let sound_infos: Vec<SoundInfo> = cached_sounds
					.iter()
					.map(|s| SoundInfo {
						sound_id: s.sound_id.clone(),
						name: s.name.clone(),
						emoji_name: s.emoji_name.clone(),
					})
					.collect();
				let sound_count = sound_infos.len();
				log::debug!("Returning {} cached sounds for guild {}", sound_count, guild_id);
				send_sounds_response(instance, sound_infos, None).await?;
				return Ok(());
			}
			drop(sounds_map);

			if let Some(error) = crate::rpc_events::soundboard_request_error().read().await.clone() {
				log::debug!("Soundboard listing unavailable: {}", error);
				send_sounds_response(instance, vec![], Some(error)).await?;
				return Ok(());
			}

			crate::rpc_events::set_pending_soundboard_guild(Some(guild_id.clone())).await;

			// No cached sounds, request from Discord
			log::debug!("No cached sounds for guild {}, requesting from Discord", guild_id);
			if let Err(e) = request_soundboard_sounds_for_guild(&guild_id).await {
				log::error!("Failed to request sounds: {}", e);
				crate::rpc_events::set_pending_soundboard_guild(None).await;
				send_sounds_response(instance, vec![], Some(e)).await?;
				return Ok(());
			}

			return Ok(());
		}

		let mut new_settings = HashMap::new();
		if let Some(sound_id) = payload.sound_id {
			new_settings.insert("sound_id".to_string(), sound_id);
		}
		if let Some(guild_id) = payload.guild_id {
			new_settings.insert("guild_id".to_string(), guild_id);
		}
		if let Some(sound_name) = payload.sound_name {
			new_settings.insert("sound_name".to_string(), sound_name);
		}

		if !new_settings.is_empty() {
			instance.set_settings(&new_settings).await?;
		}

		Ok(())
	}
}
