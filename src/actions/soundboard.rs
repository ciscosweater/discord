use crate::client::discord_client;
use crate::rpc_events::{available_soundboard_guilds, guild_request_error, soundboard_request_error, soundboard_sounds};

use std::collections::HashMap;

use discord_ipc_rust::models::send::commands::{GetSoundboardSoundsArgs, SentCommand};
use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};
use serde_json::json;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
struct SoundboardPayload {
	action: Option<String>,
	sound_id: Option<String>,
	guild_id: Option<String>,
	sound_name: Option<String>,
}

#[derive(Debug, serde::Serialize, Clone)]
pub(crate) struct SoundInfo {
	pub(crate) sound_id: String,
	pub(crate) name: String,
	pub(crate) emoji_name: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct SoundsResponse {
	pub(crate) action: String,
	pub(crate) sounds: Vec<SoundInfo>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(crate) error: Option<String>,
}

#[derive(Debug, serde::Serialize, Clone)]
pub(crate) struct GuildInfo {
	pub(crate) guild_id: String,
	pub(crate) name: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct GuildsResponse {
	pub(crate) action: String,
	pub(crate) guilds: Vec<GuildInfo>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(crate) error: Option<String>,
}

async fn request_guilds() -> Result<(), String> {
	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		return Err("Discord client not initialized".to_string());
	};

	log::debug!("Requesting guild list from Discord RPC");
	client
		.emit_command(&SentCommand::GetGuilds)
		.await
		.map_err(|error| format!("Failed to request guild list: {}", error))
}

async fn request_soundboard_sounds_for_guild(guild_id: &str) -> Result<(), String> {
	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		return Err("Discord client not initialized".to_string());
	};

	log::debug!("Requesting soundboard sounds for guild {} via Discord RPC", guild_id);
	client
		.emit_command(&SentCommand::GetSoundboardSounds(GetSoundboardSoundsArgs {
			guild_id: guild_id.to_string(),
		}))
		.await
		.map_err(|error| format!("Failed to request soundboard sounds: {}", error))
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

async fn send_guilds_response(
	instance: &Instance,
	guilds: Vec<GuildInfo>,
	error: Option<String>,
) -> OpenActionResult<()> {
	log::debug!(
		"send_guilds_response sending {} guilds directly to PI, error_present={}",
		guilds.len(),
		error.is_some()
	);
	let response = GuildsResponse {
		action: "guilds_result".to_string(),
		guilds,
		error,
	};
	instance
		.send_to_property_inspector(&serde_json::to_string(&response).unwrap())
		.await
}

async fn wait_for_guilds(instance: &Instance) -> OpenActionResult<()> {
	for _ in 0..60 {
		let guilds = available_soundboard_guilds().read().await.clone();
		if !guilds.is_empty() {
			log::debug!("wait_for_guilds found {} cached guilds", guilds.len());
			return send_guilds_response(instance, guilds, None).await;
		}

		if let Some(error) = guild_request_error().read().await.clone() {
			log::debug!("wait_for_guilds saw guild error: {}", error);
			return send_guilds_response(instance, vec![], Some(error)).await;
		}

		sleep(Duration::from_millis(250)).await;
	}

	send_guilds_response(
		instance,
		vec![],
		Some("Timed out waiting for guild list from Discord RPC.".to_string()),
	)
	.await
}

async fn wait_for_sounds(instance: &Instance, guild_id: &str) -> OpenActionResult<()> {
	for _ in 0..60 {
		let sounds_map = soundboard_sounds().read().await;
		if let Some(cached_sounds) = sounds_map.get(guild_id) {
			let sounds = cached_sounds
				.iter()
				.map(|sound| SoundInfo {
					sound_id: sound.sound_id.clone(),
					name: sound.name.clone(),
					emoji_name: sound.emoji_name.clone(),
				})
				.collect();
			drop(sounds_map);
			return send_sounds_response(instance, sounds, None).await;
		}
		drop(sounds_map);

		if let Some(error) = soundboard_request_error().read().await.clone() {
			return send_sounds_response(instance, vec![], Some(error)).await;
		}

		sleep(Duration::from_millis(250)).await;
	}

	send_sounds_response(
		instance,
		vec![],
		Some("Timed out waiting for soundboard sounds from Discord RPC.".to_string()),
	)
	.await
}

pub struct PlaySoundboardSoundAction;
#[async_trait]
impl Action for PlaySoundboardSoundAction {
	const UUID: ActionUuid = "com.elgato.discord.soundboard";
	type Settings = HashMap<String, String>;

	async fn property_inspector_did_appear(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
	) -> OpenActionResult<()> {
		let guilds = available_soundboard_guilds().read().await.clone();
		if !guilds.is_empty() {
			return send_guilds_response(instance, guilds, None).await;
		}

		match request_guilds().await {
			Ok(()) => Ok(()),
			Err(error) => send_guilds_response(instance, vec![], Some(error)).await,
		}
	}

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

		let args = if guild_id == "DEFAULT" {
			json!({
				"sound_id": sound_id,
				"guild_id": "0"
			})
		} else {
			json!({
				"sound_id": sound_id,
				"guild_id": guild_id
			})
		};
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

		match payload.action.as_deref() {
			Some("get_guilds") => {
				log::debug!("soundboard send_to_plugin handling get_guilds");
				let guilds = available_soundboard_guilds().read().await.clone();
				if !guilds.is_empty() {
					log::debug!("get_guilds found {} cached guilds immediately", guilds.len());
					send_guilds_response(instance, guilds, None).await?;
					return Ok(());
				}

				if let Some(error) = guild_request_error().read().await.clone() {
					log::debug!("get_guilds found cached guild error: {}", error);
					send_guilds_response(instance, vec![], Some(error)).await?;
					return Ok(());
				}

				if let Err(error) = request_guilds().await {
					log::debug!("get_guilds request_guilds failed immediately: {}", error);
					send_guilds_response(instance, vec![], Some(error)).await?;
					return Ok(());
				}
				log::debug!("get_guilds waiting for guild cache");
				wait_for_guilds(instance).await?;
				return Ok(());
			}
			Some("get_sounds") => {
				let guild_id = match &payload.guild_id {
					Some(id) => id.clone(),
					None => {
						log::warn!("get_sounds request but no guild_id provided");
						return Ok(());
					}
				};

				let sounds_map = soundboard_sounds().read().await;
				if let Some(cached_sounds) = sounds_map.get(&guild_id) {
					let sounds = cached_sounds
						.iter()
						.map(|sound| SoundInfo {
							sound_id: sound.sound_id.clone(),
							name: sound.name.clone(),
							emoji_name: sound.emoji_name.clone(),
						})
						.collect();
					send_sounds_response(instance, sounds, None).await?;
					return Ok(());
				}
				drop(sounds_map);

				if let Some(error) = soundboard_request_error().read().await.clone() {
					send_sounds_response(instance, vec![], Some(error)).await?;
					return Ok(());
				}

				crate::rpc_events::set_pending_soundboard_guild(Some(guild_id.clone())).await;
				if let Err(error) = request_soundboard_sounds_for_guild(&guild_id).await {
					crate::rpc_events::set_pending_soundboard_guild(None).await;
					send_sounds_response(instance, vec![], Some(error)).await?;
					return Ok(());
				}
				wait_for_sounds(instance, &guild_id).await?;
				return Ok(());
			}
			_ => {}
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
