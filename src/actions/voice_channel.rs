use crate::actions::GuildInfo;
use crate::client::discord_client;
use crate::rpc_events::{
	available_soundboard_guilds, guild_request_error, register_pending_voice_channel_request,
	voice_channel_request_error, voice_channels,
};

use std::collections::{HashMap, hash_map::DefaultHasher};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use discord_ipc_rust::models::send::commands::{
	GetChannelsArgs, SelectVoiceChannelArgs, SentCommand,
};
use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};
use tokio::sync::RwLock;
use tokio::task;
use tokio::time::{Duration, sleep};

#[derive(Debug, serde::Deserialize)]
struct VoiceChannelPayload {
	action: Option<String>,
	guild_id: Option<String>,
	guild_icon_hash: Option<String>,
	channel_id: Option<String>,
	channel_name: Option<String>,
	show_channel_title: Option<bool>,
}

#[derive(Debug, serde::Serialize, Clone)]
pub(crate) struct VoiceChannelInfo {
	pub(crate) channel_id: String,
	pub(crate) name: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ChannelsResponse {
	pub(crate) action: String,
	pub(crate) guild_id: String,
	pub(crate) channels: Vec<VoiceChannelInfo>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub(crate) error: Option<String>,
}

fn voice_channel_instance_settings() -> &'static RwLock<HashMap<String, HashMap<String, String>>> {
	static SETTINGS: OnceLock<RwLock<HashMap<String, HashMap<String, String>>>> = OnceLock::new();
	SETTINGS.get_or_init(|| RwLock::new(HashMap::new()))
}

async fn remember_instance_settings(instance: &Instance, settings: &HashMap<String, String>) {
	voice_channel_instance_settings()
		.write()
		.await
		.insert(instance.instance_id.clone(), settings.clone());
}

async fn merge_instance_settings(
	instance: &Instance,
	settings: &HashMap<String, String>,
) -> HashMap<String, String> {
	let mut registry = voice_channel_instance_settings().write().await;
	let merged = registry.entry(instance.instance_id.clone()).or_default();
	for (key, value) in settings {
		merged.insert(key.clone(), value.clone());
	}
	merged.clone()
}

async fn forget_instance_settings(instance: &Instance) {
	voice_channel_instance_settings()
		.write()
		.await
		.remove(&instance.instance_id);
}

fn channel_title_enabled(settings: &HashMap<String, String>) -> bool {
	settings
		.get("show_channel_title")
		.map(|value| value.trim())
		.map(|value| !value.eq_ignore_ascii_case("false"))
		.unwrap_or(true)
}

fn fallback_join_voice_channel_image() -> &'static str {
	"actions/voicechannel_0"
}

fn plugin_actions_dir() -> Option<PathBuf> {
	let exe_path = std::env::current_exe().ok()?;
	let exe_dir = exe_path.parent()?.to_path_buf();
	Some(exe_dir.join("actions"))
}

fn ensure_generated_dir(actions_dir: &Path) -> Option<PathBuf> {
	let generated_dir = actions_dir.join("generated");
	if let Err(error) = fs::create_dir_all(&generated_dir) {
		log::error!(
			"Failed to create generated dir {}: {}",
			generated_dir.display(),
			error
		);
		return None;
	}
	Some(generated_dir)
}

fn guild_icon_image_filename(guild_id: &str, icon_hash: &str) -> String {
	let mut hasher = DefaultHasher::new();
	"join_voice_guild_icon_v2".hash(&mut hasher);
	guild_id.hash(&mut hasher);
	icon_hash.hash(&mut hasher);
	format!("join_voice_guild_{:016x}.png", hasher.finish())
}

fn guild_icon_cache_filename(guild_id: &str, icon_hash: &str) -> String {
	let mut hasher = DefaultHasher::new();
	"guild_icon_cache_v2".hash(&mut hasher);
	guild_id.hash(&mut hasher);
	icon_hash.hash(&mut hasher);
	format!("guild_icon_{:016x}.png", hasher.finish())
}

async fn cached_guild_icon_path(guild_id: &str, icon_hash: &str) -> Option<PathBuf> {
	let actions_dir = plugin_actions_dir()?;
	let generated_dir = ensure_generated_dir(&actions_dir)?;
	let icon_path = generated_dir.join(guild_icon_cache_filename(guild_id, icon_hash));
	if icon_path.exists() {
		return Some(icon_path);
	}

	let url = format!(
		"https://cdn.discordapp.com/icons/{}/{}.png?size=128",
		guild_id, icon_hash
	);
	let response = match reqwest::get(&url).await {
		Ok(response) => response,
		Err(error) => {
			log::warn!("Failed to fetch guild icon for {}: {}", guild_id, error);
			return None;
		}
	};
	let response = match response.error_for_status() {
		Ok(response) => response,
		Err(error) => {
			log::warn!(
				"Discord CDN returned an error for guild icon {}: {}",
				guild_id,
				error
			);
			return None;
		}
	};
	let bytes = match response.bytes().await {
		Ok(bytes) => bytes,
		Err(error) => {
			log::warn!(
				"Failed to read guild icon bytes for {}: {}",
				guild_id,
				error
			);
			return None;
		}
	};
	if let Err(error) = fs::write(&icon_path, bytes.as_ref()) {
		log::warn!(
			"Failed to write guild icon cache {}: {}",
			icon_path.display(),
			error
		);
		return None;
	}

	Some(icon_path)
}

fn guild_icon_hash_from_cached_guilds(guild_id: &str, guilds: &[GuildInfo]) -> Option<String> {
	guilds
		.iter()
		.find(|guild| guild.guild_id == guild_id)
		.and_then(|guild| guild.icon_hash.as_deref())
		.map(str::trim)
		.filter(|hash| !hash.is_empty())
		.map(str::to_string)
}

async fn resolved_guild_icon_hash(settings: &HashMap<String, String>) -> Option<String> {
	if let Some(icon_hash) = settings
		.get("guild_icon_hash")
		.map(String::as_str)
		.map(str::trim)
		.filter(|hash| !hash.is_empty())
	{
		return Some(icon_hash.to_string());
	}

	let guild_id = settings
		.get("guild_id")
		.map(String::as_str)
		.map(str::trim)
		.filter(|guild_id| !guild_id.is_empty())?;
	let guilds = available_soundboard_guilds().read().await;
	guild_icon_hash_from_cached_guilds(guild_id, &guilds)
}

fn compose_guild_icon_image(icon_path: &Path, guild_id: &str, icon_hash: &str) -> Option<String> {
	let actions_dir = plugin_actions_dir()?;
	let generated_dir = ensure_generated_dir(&actions_dir)?;
	let filename = guild_icon_image_filename(guild_id, icon_hash);
	let output_path = generated_dir.join(&filename);
	let image_path = format!("actions/generated/{}", filename.trim_end_matches(".png"));
	if output_path.exists() {
		return Some(image_path);
	}

	let blank_svg_path = actions_dir.join("blank.svg");
	let temp_icon_path = generated_dir.join(format!(
		"join_voice_guild_icon_masked_{}.png",
		uuid::Uuid::new_v4()
	));
	let temp_frame_path = generated_dir.join(format!(
		"join_voice_guild_icon_frame_{}.png",
		uuid::Uuid::new_v4()
	));

	let icon_mask = Command::new("convert")
		.arg(icon_path)
		.arg("-resize")
		.arg("92x92^")
		.arg("-gravity")
		.arg("center")
		.arg("-crop")
		.arg("92x92+0+0")
		.arg("+repage")
		.arg("(")
		.arg("-size")
		.arg("92x92")
		.arg("xc:none")
		.arg("-fill")
		.arg("white")
		.arg("-draw")
		.arg("roundrectangle 0,0 91,91 12,12")
		.arg(")")
		.arg("-alpha")
		.arg("off")
		.arg("-compose")
		.arg("CopyOpacity")
		.arg("-composite")
		.arg(format!("png32:{}", temp_icon_path.display()))
		.output();
	match icon_mask {
		Ok(output) if output.status.success() => {}
		Ok(output) => {
			log::error!(
				"Failed to mask guild icon {}: status={} stderr={}",
				icon_path.display(),
				output.status,
				String::from_utf8_lossy(&output.stderr)
			);
			return None;
		}
		Err(error) => {
			log::error!("Failed to start convert for guild icon mask: {}", error);
			return None;
		}
	}

	let frame_render = Command::new("convert")
		.arg("-size")
		.arg("120x120")
		.arg("xc:none")
		.arg("-stroke")
		.arg("white")
		.arg("-strokewidth")
		.arg("4")
		.arg("-fill")
		.arg("none")
		.arg("-draw")
		.arg("roundrectangle 8,8 111,111 13,13")
		.arg(format!("png32:{}", temp_frame_path.display()))
		.output();
	match frame_render {
		Ok(output) if output.status.success() => {}
		Ok(output) => {
			log::error!(
				"Failed to render guild icon frame: status={} stderr={}",
				output.status,
				String::from_utf8_lossy(&output.stderr)
			);
			let _ = fs::remove_file(&temp_icon_path);
			return None;
		}
		Err(error) => {
			log::error!("Failed to start convert for guild icon frame: {}", error);
			let _ = fs::remove_file(&temp_icon_path);
			return None;
		}
	}

	let compose = Command::new("convert")
		.arg(&blank_svg_path)
		.arg(&temp_icon_path)
		.arg("-gravity")
		.arg("center")
		.arg("-geometry")
		.arg("+0+0")
		.arg("-composite")
		.arg(&temp_frame_path)
		.arg("-gravity")
		.arg("center")
		.arg("-geometry")
		.arg("+0+0")
		.arg("-composite")
		.arg(&output_path)
		.output();
	let result = match compose {
		Ok(output) if output.status.success() => Some(image_path),
		Ok(output) => {
			log::error!(
				"Failed to compose guild icon image {}: status={} stderr={}",
				output_path.display(),
				output.status,
				String::from_utf8_lossy(&output.stderr)
			);
			None
		}
		Err(error) => {
			log::error!(
				"Failed to start convert for guild icon composition: {}",
				error
			);
			None
		}
	};

	let _ = fs::remove_file(&temp_icon_path);
	let _ = fs::remove_file(&temp_frame_path);
	result
}

async fn render_join_voice_channel_image(settings: &HashMap<String, String>) -> Option<String> {
	let guild_id = settings
		.get("guild_id")
		.map(String::as_str)
		.map(str::trim)
		.filter(|guild_id| !guild_id.is_empty())?
		.to_string();
	let icon_hash = resolved_guild_icon_hash(settings).await?;
	let icon_path = cached_guild_icon_path(&guild_id, &icon_hash).await?;
	task::spawn_blocking(move || compose_guild_icon_image(&icon_path, &guild_id, &icon_hash))
		.await
		.ok()
		.flatten()
}

async fn update_join_voice_channel_button(
	instance: &Instance,
	settings: &HashMap<String, String>,
) -> OpenActionResult<()> {
	let image = render_join_voice_channel_image(settings)
		.await
		.unwrap_or_else(|| fallback_join_voice_channel_image().to_string());
	instance.set_image(Some(image), None).await?;
	let title = if channel_title_enabled(settings) {
		settings
			.get("channel_name")
			.map(|value| value.trim())
			.filter(|value| !value.is_empty())
			.map(str::to_string)
			.unwrap_or_default()
	} else {
		String::new()
	};
	instance.set_title(Some(title), None).await?;
	Ok(())
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

fn voice_channel_guilds(guilds: Vec<GuildInfo>) -> Vec<GuildInfo> {
	guilds
		.into_iter()
		.filter(|guild| guild.guild_id != "DEFAULT")
		.collect()
}

async fn request_channels_for_guild(instance: &Instance, guild_id: &str) -> Result<(), String> {
	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		return Err("Discord client not initialized".to_string());
	};

	*voice_channel_request_error().write().await = None;
	let nonce = client
		.emit_command_with_nonce(&SentCommand::GetChannels(GetChannelsArgs {
			guild_id: guild_id.to_string(),
		}))
		.await
		.map_err(|error| format!("Failed to request guild channels: {}", error))?;
	drop(client_lock);

	register_pending_voice_channel_request(&nonce, &instance.instance_id, guild_id).await;
	Ok(())
}

async fn send_guilds_response(
	instance: &Instance,
	guilds: Vec<GuildInfo>,
	error: Option<String>,
) -> OpenActionResult<()> {
	let response = serde_json::json!({
		"action": "guilds_result",
		"guilds": guilds,
		"error": error,
	});
	instance
		.send_to_property_inspector(&response.to_string())
		.await
}

async fn send_channels_response(
	instance: &Instance,
	guild_id: &str,
	channels: Vec<VoiceChannelInfo>,
	error: Option<String>,
) -> OpenActionResult<()> {
	let payload = match serde_json::to_string(&ChannelsResponse {
		action: "channels_result".to_string(),
		guild_id: guild_id.to_string(),
		channels,
		error,
	}) {
		Ok(payload) => payload,
		Err(error) => {
			log::error!("Failed to serialize voice channel response: {}", error);
			return Ok(());
		}
	};
	instance.send_to_property_inspector(&payload).await
}

async fn wait_for_guilds(instance: &Instance) -> OpenActionResult<()> {
	for _ in 0..60 {
		let raw_guilds = available_soundboard_guilds().read().await.clone();
		if !raw_guilds.is_empty() {
			let guilds = voice_channel_guilds(raw_guilds);
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

async fn wait_for_channels(instance: &Instance, guild_id: &str) -> OpenActionResult<()> {
	for _ in 0..60 {
		let channels_map = voice_channels().read().await;
		if let Some(cached_channels) = channels_map.get(guild_id) {
			return send_channels_response(instance, guild_id, cached_channels.clone(), None).await;
		}
		drop(channels_map);

		if let Some(error) = voice_channel_request_error().read().await.clone() {
			return send_channels_response(instance, guild_id, vec![], Some(error)).await;
		}

		sleep(Duration::from_millis(250)).await;
	}

	send_channels_response(
		instance,
		guild_id,
		vec![],
		Some("Timed out waiting for guild channels from Discord RPC.".to_string()),
	)
	.await
}

async fn update_voice_channel(
	instance: &Instance,
	args: SelectVoiceChannelArgs,
) -> OpenActionResult<()> {
	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		log::error!("Discord client not initialized");
		instance.show_alert().await?;
		return Ok(());
	};

	match client
		.emit_command(&SentCommand::SelectVoiceChannel(args))
		.await
	{
		Ok(_) => {}
		Err(error) => {
			log::error!("Failed to select voice channel: {}", error);
			instance.show_alert().await?;
		}
	}

	Ok(())
}

pub struct SelectVoiceChannelAction;
#[async_trait]
impl Action for SelectVoiceChannelAction {
	const UUID: ActionUuid = "me.amankhanna.oadiscord.selectvoicechannel";
	type Settings = HashMap<String, String>;

	async fn will_appear(
		&self,
		instance: &Instance,
		settings: &Self::Settings,
	) -> OpenActionResult<()> {
		remember_instance_settings(instance, settings).await;
		update_join_voice_channel_button(instance, settings).await
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
		update_join_voice_channel_button(instance, settings).await
	}

	async fn property_inspector_did_appear(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
	) -> OpenActionResult<()> {
		let raw_guilds = available_soundboard_guilds().read().await.clone();
		if !raw_guilds.is_empty() {
			let guilds = voice_channel_guilds(raw_guilds);
			return send_guilds_response(instance, guilds, None).await;
		}

		match request_guilds().await {
			Ok(()) => wait_for_guilds(instance).await,
			Err(error) => send_guilds_response(instance, vec![], Some(error)).await,
		}
	}

	async fn key_up(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		let channel_id = settings
			.get("channel_id")
			.cloned()
			.map(|value| value.trim().to_string())
			.filter(|value| !value.is_empty());

		if let Some(channel_id) = channel_id {
			update_voice_channel(
				instance,
				SelectVoiceChannelArgs {
					channel_id: Some(channel_id),
					force: Some(true),
					timeout: None,
					navigate: None,
				},
			)
			.await
		} else {
			log::debug!("SelectVoiceChannel settings received: {:?}", settings);
			log::error!("No channel_id provided in settings");
			instance.show_alert().await?;
			Ok(())
		}
	}

	async fn send_to_plugin(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
		payload: &serde_json::Value,
	) -> OpenActionResult<()> {
		let payload: VoiceChannelPayload = match serde_json::from_value(payload.clone()) {
			Ok(payload) => payload,
			Err(error) => {
				log::error!("Failed to parse voice channel payload: {}", error);
				return Ok(());
			}
		};

		match payload.action.as_deref() {
			Some("get_guilds") => {
				if let Err(error) = request_guilds().await {
					return send_guilds_response(instance, vec![], Some(error)).await;
				}

				return wait_for_guilds(instance).await;
			}
			Some("get_channels") => {
				let guild_id = match payload.guild_id {
					Some(guild_id) if !guild_id.trim().is_empty() => guild_id.trim().to_string(),
					_ => return Ok(()),
				};

				if let Err(error) = request_channels_for_guild(instance, &guild_id).await {
					return send_channels_response(instance, &guild_id, vec![], Some(error)).await;
				}

				return wait_for_channels(instance, &guild_id).await;
			}
			_ => {}
		}

		let mut new_settings = HashMap::new();
		if let Some(guild_id) = payload.guild_id {
			new_settings.insert("guild_id".to_string(), guild_id);
		}
		if let Some(guild_icon_hash) = payload.guild_icon_hash {
			new_settings.insert("guild_icon_hash".to_string(), guild_icon_hash);
		}
		if let Some(channel_id) = payload.channel_id {
			new_settings.insert("channel_id".to_string(), channel_id);
		}
		if let Some(channel_name) = payload.channel_name {
			new_settings.insert("channel_name".to_string(), channel_name);
		}
		if let Some(show_channel_title) = payload.show_channel_title {
			new_settings.insert(
				"show_channel_title".to_string(),
				show_channel_title.to_string(),
			);
		}

		if !new_settings.is_empty() {
			let merged_settings = merge_instance_settings(instance, &new_settings).await;
			instance.set_settings(&merged_settings).await?;
			update_join_voice_channel_button(instance, &merged_settings).await?;
		}

		Ok(())
	}
}

pub struct LeaveVoiceChannelAction;
#[async_trait]
impl Action for LeaveVoiceChannelAction {
	const UUID: ActionUuid = "me.amankhanna.oadiscord.leavevoicechannel";
	type Settings = HashMap<String, String>;

	async fn key_up(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
	) -> OpenActionResult<()> {
		update_voice_channel(
			instance,
			SelectVoiceChannelArgs {
				channel_id: None,
				force: Some(true),
				timeout: None,
				navigate: None,
			},
		)
		.await
	}
}

#[cfg(test)]
mod tests {
	use super::{
		channel_title_enabled, guild_icon_hash_from_cached_guilds, guild_icon_image_filename,
	};
	use crate::actions::GuildInfo;
	use std::collections::HashMap;

	#[test]
	fn channel_title_enabled_defaults_to_true() {
		assert!(channel_title_enabled(&HashMap::new()));
	}

	#[test]
	fn channel_title_enabled_respects_false_string() {
		let mut settings = HashMap::new();
		settings.insert("show_channel_title".to_string(), "false".to_string());
		assert!(!channel_title_enabled(&settings));
	}

	#[test]
	fn guild_icon_image_filename_changes_with_icon_hash() {
		let first = guild_icon_image_filename("1", "icon-a");
		let second = guild_icon_image_filename("1", "icon-b");
		assert_ne!(first, second);
	}

	#[test]
	fn guild_icon_hash_from_cached_guilds_returns_matching_hash() {
		let guilds = vec![
			GuildInfo {
				guild_id: "1".to_string(),
				name: "Alpha".to_string(),
				icon_hash: Some("hash-a".to_string()),
			},
			GuildInfo {
				guild_id: "2".to_string(),
				name: "Beta".to_string(),
				icon_hash: None,
			},
		];

		assert_eq!(
			guild_icon_hash_from_cached_guilds("1", &guilds),
			Some("hash-a".to_string())
		);
		assert_eq!(guild_icon_hash_from_cached_guilds("2", &guilds), None);
		assert_eq!(guild_icon_hash_from_cached_guilds("3", &guilds), None);
	}
}
