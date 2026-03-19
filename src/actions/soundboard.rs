use crate::client::discord_client;
use crate::rpc_events::{
	available_soundboard_guilds, guild_request_error, register_pending_soundboard_request,
	soundboard_request_error, soundboard_sounds,
};

use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use discord_ipc_rust::models::send::commands::{GetSoundboardSoundsArgs, SentCommand};
use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};
use serde_json::json;
use tokio::sync::RwLock;
use tokio::task;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
struct SoundboardPayload {
	action: Option<String>,
	sound_id: Option<String>,
	guild_id: Option<String>,
	sound_name: Option<String>,
	emoji_name: Option<String>,
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
	pub(crate) guild_id: String,
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

	client
		.emit_command(&SentCommand::GetGuilds)
		.await
		.map_err(|error| format!("Failed to request guild list: {}", error))
}

fn soundboard_instance_settings() -> &'static RwLock<HashMap<String, HashMap<String, String>>> {
	static SETTINGS: OnceLock<RwLock<HashMap<String, HashMap<String, String>>>> = OnceLock::new();
	SETTINGS.get_or_init(|| RwLock::new(HashMap::new()))
}

async fn remember_instance_settings(instance: &Instance, settings: &HashMap<String, String>) {
	soundboard_instance_settings()
		.write()
		.await
		.insert(instance.instance_id.clone(), settings.clone());
}

async fn merge_instance_settings(
	instance: &Instance,
	settings: &HashMap<String, String>,
) -> HashMap<String, String> {
	let mut registry = soundboard_instance_settings().write().await;
	let merged = registry.entry(instance.instance_id.clone()).or_default();
	for (key, value) in settings {
		merged.insert(key.clone(), value.clone());
	}
	merged.clone()
}

async fn forget_instance_settings(instance: &Instance) {
	soundboard_instance_settings()
		.write()
		.await
		.remove(&instance.instance_id);
}

async fn request_soundboard_sounds_for_guild(
	instance: &Instance,
	guild_id: &str,
) -> Result<(), String> {
	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		return Err("Discord client not initialized".to_string());
	};

	*soundboard_request_error().write().await = None;
	let nonce = client
		.emit_command_with_nonce(&SentCommand::GetSoundboardSounds(GetSoundboardSoundsArgs {
			guild_id: guild_id.to_string(),
		}))
		.await
		.map_err(|error| format!("Failed to request soundboard sounds: {}", error))?;
	drop(client_lock);

	register_pending_soundboard_request(&nonce, &instance.instance_id, guild_id).await;
	Ok(())
}

async fn send_sounds_response(
	instance: &Instance,
	guild_id: &str,
	sounds: Vec<SoundInfo>,
	error: Option<String>,
) -> OpenActionResult<()> {
	let response = SoundsResponse {
		action: "sounds_result".to_string(),
		guild_id: guild_id.to_string(),
		sounds,
		error,
	};
	let payload = match serde_json::to_string(&response) {
		Ok(payload) => payload,
		Err(error) => {
			log::error!("Failed to serialize soundboard sounds response: {}", error);
			return Ok(());
		}
	};
	instance.send_to_property_inspector(&payload).await
}

async fn send_guilds_response(
	instance: &Instance,
	guilds: Vec<GuildInfo>,
	error: Option<String>,
) -> OpenActionResult<()> {
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
			return send_guilds_response(instance, guilds, None).await;
		}

		if let Some(error) = guild_request_error().read().await.clone() {
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
			return send_sounds_response(instance, guild_id, sounds, None).await;
		}
		drop(sounds_map);

		if let Some(error) = soundboard_request_error().read().await.clone() {
			return send_sounds_response(instance, guild_id, vec![], Some(error)).await;
		}

		sleep(Duration::from_millis(250)).await;
	}

	send_sounds_response(
		instance,
		guild_id,
		vec![],
		Some("Timed out waiting for soundboard sounds from Discord RPC.".to_string()),
	)
	.await
}

fn escape_xml_text(input: &str) -> String {
	let mut escaped = String::with_capacity(input.len());
	for ch in input.chars() {
		match ch {
			'&' => escaped.push_str("&amp;"),
			'<' => escaped.push_str("&lt;"),
			'>' => escaped.push_str("&gt;"),
			'"' => escaped.push_str("&quot;"),
			'\'' => escaped.push_str("&apos;"),
			_ => escaped.push(ch),
		}
	}
	escaped
}

fn escape_pango_text(input: &str) -> String {
	escape_xml_text(input)
}

fn plugin_actions_dir() -> Option<PathBuf> {
	let exe_path = std::env::current_exe().ok()?;
	let exe_dir = exe_path.parent()?.to_path_buf();
	Some(exe_dir.join("actions"))
}

fn write_soundboard_image(emoji_name: Option<&str>) -> Option<String> {
	let trimmed_emoji = emoji_name.map(str::trim).filter(|emoji| !emoji.is_empty());
	if let Some(emoji) = trimmed_emoji {
		let actions_dir = plugin_actions_dir()?;
		let generated_dir = actions_dir.join("generated");
		if let Err(error) = fs::create_dir_all(&generated_dir) {
			log::error!(
				"Failed to create generated dir {}: {}",
				generated_dir.display(),
				error
			);
			return None;
		}

		let mut hasher = std::collections::hash_map::DefaultHasher::new();
		emoji.hash(&mut hasher);
		let hash = hasher.finish();
		let filename = format!("soundboard_{hash:016x}.png");
		let output_path = generated_dir.join(&filename);
		let image_path = format!("actions/generated/{}", filename.trim_end_matches(".png"));
		if output_path.exists() {
			return Some(image_path);
		}
		let temp_emoji_path = generated_dir.join(format!(
			"soundboard_{hash:016x}_emoji_{}.png",
			Uuid::new_v4()
		));
		let blank_svg_path = actions_dir.join("blank.svg");
		let pango_markup = format!(
			r#"<span font="Noto Color Emoji 70">{}</span>"#,
			escape_pango_text(emoji)
		);

		let emoji_render = Command::new("convert")
			.arg("-background")
			.arg("none")
			.arg(format!("pango:{pango_markup}"))
			.arg("-trim")
			.arg("+repage")
			.arg(&temp_emoji_path)
			.output();
		match emoji_render {
			Ok(output) if output.status.success() => {}
			Ok(output) => {
				log::error!(
					"Failed to render emoji png for {:?}: status={} stderr={}",
					emoji,
					output.status,
					String::from_utf8_lossy(&output.stderr)
				);
				return None;
			}
			Err(error) => {
				log::error!("Failed to start convert for emoji render: {}", error);
				return None;
			}
		}

		let compose = Command::new("convert")
			.arg(&blank_svg_path)
			.arg(&temp_emoji_path)
			.arg("-gravity")
			.arg("center")
			.arg("-geometry")
			.arg("+0-2")
			.arg("-composite")
			.arg(&output_path)
			.output();
		match compose {
			Ok(output) if output.status.success() => {}
			Ok(output) => {
				log::error!(
					"Failed to compose png {}: status={} stderr={}",
					output_path.display(),
					output.status,
					String::from_utf8_lossy(&output.stderr)
				);
				let _ = fs::remove_file(&temp_emoji_path);
				return None;
			}
			Err(error) => {
				log::error!("Failed to start convert for composition: {}", error);
				let _ = fs::remove_file(&temp_emoji_path);
				return None;
			}
		}

		let _ = fs::remove_file(&temp_emoji_path);
		if !output_path.exists() {
			log::error!(
				"Expected composed png not found after convert: {}",
				output_path.display()
			);
			return None;
		}

		Some(image_path)
	} else {
		Some("actions/blank".to_string())
	}
}

async fn update_soundboard_button(
	instance: &Instance,
	settings: &HashMap<String, String>,
) -> OpenActionResult<()> {
	let emoji_name = settings.get("emoji_name").cloned();
	let image = task::spawn_blocking(move || write_soundboard_image(emoji_name.as_deref()))
		.await
		.ok()
		.flatten()
		.unwrap_or_else(|| "actions/blank".to_string());
	instance.set_image(Some(image), None).await?;
	instance.set_title(None::<String>, None).await?;
	Ok(())
}

pub struct PlaySoundboardSoundAction;
#[async_trait]
impl Action for PlaySoundboardSoundAction {
	const UUID: ActionUuid = "com.elgato.discord.soundboard";
	type Settings = HashMap<String, String>;

	async fn will_appear(
		&self,
		instance: &Instance,
		settings: &Self::Settings,
	) -> OpenActionResult<()> {
		remember_instance_settings(instance, settings).await;
		update_soundboard_button(instance, settings).await
	}

	async fn will_disappear(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
	) -> OpenActionResult<()> {
		forget_instance_settings(instance).await;
		Ok(())
	}

	async fn did_receive_settings(
		&self,
		instance: &Instance,
		settings: &Self::Settings,
	) -> OpenActionResult<()> {
		remember_instance_settings(instance, settings).await;
		update_soundboard_button(instance, settings).await
	}

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

		let sound_id = settings
			.get("sound_id")
			.cloned()
			.map(|value| value.trim().to_string())
			.filter(|value| !value.is_empty());
		let guild_id = settings
			.get("guild_id")
			.cloned()
			.map(|value| value.trim().to_string())
			.filter(|value| !value.is_empty());

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
				let guilds = available_soundboard_guilds().read().await.clone();
				if !guilds.is_empty() {
					send_guilds_response(instance, guilds, None).await?;
					return Ok(());
				}

				if let Some(error) = guild_request_error().read().await.clone() {
					send_guilds_response(instance, vec![], Some(error)).await?;
					return Ok(());
				}

				if let Err(error) = request_guilds().await {
					send_guilds_response(instance, vec![], Some(error)).await?;
					return Ok(());
				}
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
					send_sounds_response(instance, &guild_id, sounds, None).await?;
					return Ok(());
				}
				drop(sounds_map);

				if let Some(error) = soundboard_request_error().read().await.clone() {
					send_sounds_response(instance, &guild_id, vec![], Some(error)).await?;
					return Ok(());
				}

				if let Err(error) = request_soundboard_sounds_for_guild(instance, &guild_id).await {
					send_sounds_response(instance, &guild_id, vec![], Some(error)).await?;
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
		if let Some(emoji_name) = payload.emoji_name {
			new_settings.insert("emoji_name".to_string(), emoji_name);
		}

		if !new_settings.is_empty() {
			let merged_settings = merge_instance_settings(instance, &new_settings).await;
			instance.set_settings(&merged_settings).await?;
			update_soundboard_button(instance, &merged_settings).await?;
		}

		Ok(())
	}
}
