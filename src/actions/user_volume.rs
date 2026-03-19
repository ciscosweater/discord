use crate::client::discord_client;
use crate::rpc_events::{
	VoiceParticipant, apply_local_user_voice_update, current_subscribed_voice_channel,
	current_voice_participant, current_voice_participants,
};

use std::collections::{HashMap, hash_map::DefaultHasher};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use discord_ipc_rust::models::send::commands::{SentCommand, SetUserVoiceSettingsArgs};
use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait, visible_instances};
use tokio::sync::RwLock;
use tokio::task;

#[derive(Debug, serde::Deserialize)]
struct UserVolumePayload {
	action: Option<String>,
	user_id: Option<String>,
	mode: Option<String>,
	mute_type: Option<String>,
	adjust_value: Option<i32>,
	set_value: Option<i32>,
}

#[derive(Debug, serde::Serialize)]
struct UsersResponse {
	action: String,
	users: Vec<crate::rpc_events::VoiceParticipant>,
	#[serde(skip_serializing_if = "Option::is_none")]
	error: Option<String>,
}

fn user_volume_instance_settings() -> &'static RwLock<HashMap<String, HashMap<String, String>>> {
	static SETTINGS: OnceLock<RwLock<HashMap<String, HashMap<String, String>>>> = OnceLock::new();
	SETTINGS.get_or_init(|| RwLock::new(HashMap::new()))
}

async fn remember_instance_settings(instance: &Instance, settings: &HashMap<String, String>) {
	user_volume_instance_settings()
		.write()
		.await
		.insert(instance.instance_id.clone(), settings.clone());
}

async fn merge_instance_settings(
	instance: &Instance,
	settings: &HashMap<String, String>,
) -> HashMap<String, String> {
	let mut registry = user_volume_instance_settings().write().await;
	let merged = registry.entry(instance.instance_id.clone()).or_default();
	for (key, value) in settings {
		merged.insert(key.clone(), value.clone());
	}
	merged.clone()
}

async fn forget_instance_settings(instance: &Instance) {
	user_volume_instance_settings()
		.write()
		.await
		.remove(&instance.instance_id);
}

async fn remembered_instance_settings(instance_id: &str) -> Option<HashMap<String, String>> {
	user_volume_instance_settings()
		.read()
		.await
		.get(instance_id)
		.cloned()
}

fn fallback_image_for_controller(controller: &str) -> &'static str {
	if controller == "Encoder" {
		"actions/volumeControl"
	} else {
		"actions/userVolumeControl"
	}
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

fn user_volume_image_filename(user_id: &str, avatar_hash: &str, muted: bool) -> String {
	let mut hasher = DefaultHasher::new();
	"v4".hash(&mut hasher);
	user_id.hash(&mut hasher);
	avatar_hash.hash(&mut hasher);
	muted.hash(&mut hasher);
	format!("user_volume_{:016x}.png", hasher.finish())
}

fn avatar_cache_filename(user_id: &str, avatar_hash: &str) -> String {
	let mut hasher = DefaultHasher::new();
	user_id.hash(&mut hasher);
	avatar_hash.hash(&mut hasher);
	format!("user_avatar_{:016x}.png", hasher.finish())
}

async fn cached_avatar_path(participant: &VoiceParticipant) -> Option<PathBuf> {
	let avatar_hash = participant.avatar_hash.as_deref()?.trim();
	if avatar_hash.is_empty() {
		return None;
	}

	let actions_dir = plugin_actions_dir()?;
	let generated_dir = ensure_generated_dir(&actions_dir)?;
	let avatar_path = generated_dir.join(avatar_cache_filename(&participant.user_id, avatar_hash));
	if avatar_path.exists() {
		return Some(avatar_path);
	}

	let url = format!(
		"https://cdn.discordapp.com/avatars/{}/{}.png?size=128",
		participant.user_id, avatar_hash
	);
	let response = match reqwest::get(&url).await {
		Ok(response) => response,
		Err(error) => {
			log::warn!(
				"Failed to fetch avatar for {}: {}",
				participant.user_id,
				error
			);
			return None;
		}
	};
	let response = match response.error_for_status() {
		Ok(response) => response,
		Err(error) => {
			log::warn!(
				"Discord CDN returned an error for avatar {}: {}",
				participant.user_id,
				error
			);
			return None;
		}
	};
	let bytes = match response.bytes().await {
		Ok(bytes) => bytes,
		Err(error) => {
			log::warn!(
				"Failed to read avatar bytes for {}: {}",
				participant.user_id,
				error
			);
			return None;
		}
	};
	if let Err(error) = fs::write(&avatar_path, bytes.as_ref()) {
		log::warn!(
			"Failed to write avatar cache {}: {}",
			avatar_path.display(),
			error
		);
		return None;
	}

	Some(avatar_path)
}

fn compose_user_volume_image(
	avatar_path: &Path,
	user_id: &str,
	avatar_hash: &str,
	muted: bool,
) -> Option<String> {
	let actions_dir = plugin_actions_dir()?;
	let generated_dir = ensure_generated_dir(&actions_dir)?;
	let filename = user_volume_image_filename(user_id, avatar_hash, muted);
	let output_path = generated_dir.join(&filename);
	let image_path = format!("actions/generated/{}", filename.trim_end_matches(".png"));
	if output_path.exists() {
		return Some(image_path);
	}

	let blank_svg_path = actions_dir.join("blank.svg");
	let badge_path = if muted {
		actions_dir.join("user_volume_muted_badge.svg")
	} else {
		actions_dir.join("user_volume_unmuted_badge.svg")
	};
	let temp_avatar_path = generated_dir.join(format!(
		"user_volume_avatar_masked_{}.png",
		uuid::Uuid::new_v4()
	));
	let temp_frame_path = generated_dir.join(format!(
		"user_volume_avatar_frame_{}.png",
		uuid::Uuid::new_v4()
	));
	let temp_badge_path =
		generated_dir.join(format!("user_volume_badge_{}.png", uuid::Uuid::new_v4()));

	let avatar_mask = Command::new("convert")
		.arg(avatar_path)
		.arg("-resize")
		.arg("104x104^")
		.arg("-gravity")
		.arg("center")
		.arg("-crop")
		.arg("104x104+0+0")
		.arg("+repage")
		.arg("(")
		.arg("-size")
		.arg("104x104")
		.arg("xc:none")
		.arg("-fill")
		.arg("white")
		.arg("-draw")
		.arg("circle 52,52 52,3")
		.arg(")")
		.arg("-alpha")
		.arg("off")
		.arg("-compose")
		.arg("CopyOpacity")
		.arg("-composite")
		.arg(&temp_avatar_path)
		.output();
	match avatar_mask {
		Ok(output) if output.status.success() => {}
		Ok(output) => {
			log::error!(
				"Failed to mask avatar {}: status={} stderr={}",
				avatar_path.display(),
				output.status,
				String::from_utf8_lossy(&output.stderr)
			);
			return None;
		}
		Err(error) => {
			log::error!("Failed to start convert for avatar mask: {}", error);
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
		.arg("circle 60,60 60,4")
		.arg(format!("png32:{}", temp_frame_path.display()))
		.output();
	match frame_render {
		Ok(output) if output.status.success() => {}
		Ok(output) => {
			log::error!(
				"Failed to render avatar frame: status={} stderr={}",
				output.status,
				String::from_utf8_lossy(&output.stderr)
			);
			let _ = fs::remove_file(&temp_avatar_path);
			return None;
		}
		Err(error) => {
			log::error!("Failed to start convert for avatar frame: {}", error);
			let _ = fs::remove_file(&temp_avatar_path);
			return None;
		}
	}

	let badge_render = Command::new("convert")
		.arg("-background")
		.arg("none")
		.arg(&badge_path)
		.arg("-resize")
		.arg("46x46")
		.arg(format!("png32:{}", temp_badge_path.display()))
		.output();
	match badge_render {
		Ok(output) if output.status.success() => {}
		Ok(output) => {
			log::error!(
				"Failed to rasterize badge {}: status={} stderr={}",
				badge_path.display(),
				output.status,
				String::from_utf8_lossy(&output.stderr)
			);
			let _ = fs::remove_file(&temp_avatar_path);
			let _ = fs::remove_file(&temp_frame_path);
			return None;
		}
		Err(error) => {
			log::error!("Failed to start convert for badge rasterization: {}", error);
			let _ = fs::remove_file(&temp_avatar_path);
			let _ = fs::remove_file(&temp_frame_path);
			return None;
		}
	}

	let compose = Command::new("convert")
		.arg(&blank_svg_path)
		.arg(&temp_frame_path)
		.arg("-gravity")
		.arg("center")
		.arg("-geometry")
		.arg("+0+1")
		.arg("-composite")
		.arg(&temp_avatar_path)
		.arg("-gravity")
		.arg("center")
		.arg("-geometry")
		.arg("+0+1")
		.arg("-composite")
		.arg(&temp_badge_path)
		.arg("-gravity")
		.arg("southeast")
		.arg("-geometry")
		.arg("+3+3")
		.arg("-composite")
		.arg(&output_path)
		.output();
	let result = match compose {
		Ok(output) if output.status.success() => Some(image_path),
		Ok(output) => {
			log::error!(
				"Failed to compose user volume image {}: status={} stderr={}",
				output_path.display(),
				output.status,
				String::from_utf8_lossy(&output.stderr)
			);
			None
		}
		Err(error) => {
			log::error!(
				"Failed to start convert for user volume composition: {}",
				error
			);
			None
		}
	};

	let _ = fs::remove_file(&temp_avatar_path);
	let _ = fs::remove_file(&temp_frame_path);
	let _ = fs::remove_file(&temp_badge_path);
	result
}

async fn render_user_volume_image(participant: &VoiceParticipant) -> Option<String> {
	let avatar_hash = participant.avatar_hash.as_deref()?.trim().to_string();
	if avatar_hash.is_empty() {
		return None;
	}

	let avatar_path = cached_avatar_path(participant).await?;
	let user_id = participant.user_id.clone();
	let muted = participant.mute;
	task::spawn_blocking(move || {
		compose_user_volume_image(&avatar_path, &user_id, &avatar_hash, muted)
	})
	.await
	.ok()
	.flatten()
}

async fn update_user_volume_image(
	instance: &Instance,
	settings: &HashMap<String, String>,
) -> OpenActionResult<()> {
	let fallback = fallback_image_for_controller(&instance.controller).to_string();
	let user_id = settings
		.get("user_id")
		.map(String::as_str)
		.unwrap_or("")
		.trim()
		.to_string();
	if user_id.is_empty() {
		instance.set_image(Some(fallback), None).await?;
		return Ok(());
	}

	let image = match current_voice_participant(&user_id).await {
		Some(participant) => render_user_volume_image(&participant)
			.await
			.unwrap_or(fallback),
		None => fallback,
	};
	instance.set_image(Some(image), None).await?;
	Ok(())
}

pub async fn refresh_user_volume_instances() {
	for action_uuid in [
		UserVolumeControlButtonAction::UUID,
		UserVolumeControlDialAction::UUID,
	] {
		for instance in visible_instances(action_uuid).await {
			let settings = remembered_instance_settings(&instance.instance_id)
				.await
				.unwrap_or_default();
			if let Err(error) = update_user_volume_image(&instance, &settings).await {
				log::error!(
					"Failed to refresh user volume image for {}: {}",
					instance.instance_id,
					error
				);
			}
		}
	}
}

async fn update_user_voice_setting(
	instance: &Instance,
	args: SetUserVoiceSettingsArgs,
) -> OpenActionResult<()> {
	let requested_user_id = args.user_id.clone();
	let requested_volume = args.volume;
	let requested_mute = args.mute;

	let mut client_lock = discord_client().write().await;
	let Some(client) = client_lock.as_mut() else {
		log::error!("Discord client not initialized");
		instance.show_alert().await?;
		return Ok(());
	};

	match client
		.emit_command(&SentCommand::SetUserVoiceSettings(args))
		.await
	{
		Ok(_) => {
			drop(client_lock);
			apply_local_user_voice_update(&requested_user_id, requested_volume, requested_mute)
				.await;
			instance.show_ok().await?;
		}
		Err(e) => {
			log::error!("Failed to update user voice settings: {}", e);
			instance.show_alert().await?;
		}
	}

	Ok(())
}

pub struct UserVolumeControlButtonAction;
#[async_trait]
impl Action for UserVolumeControlButtonAction {
	const UUID: ActionUuid = "me.amankhanna.oadiscord.uservolumecontrolbutton";
	type Settings = HashMap<String, String>;

	async fn will_appear(
		&self,
		instance: &Instance,
		settings: &Self::Settings,
	) -> OpenActionResult<()> {
		remember_instance_settings(instance, settings).await;
		update_user_volume_image(instance, settings).await
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
		update_user_volume_image(instance, settings).await
	}

	async fn property_inspector_did_appear(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
	) -> OpenActionResult<()> {
		log::info!("UserVolumeControlButton property inspector appeared");
		send_users_response(instance).await
	}

	async fn key_up(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		log::debug!("UserVolumeControlButton settings: {:?}", settings);

		let user_id = settings.get("user_id").cloned();
		let mode = settings
			.get("mode")
			.cloned()
			.unwrap_or_else(|| "mute".to_string());
		let mute_type = settings
			.get("mute_type")
			.cloned()
			.unwrap_or_else(|| "toggle".to_string());

		let Some(user_id) = user_id else {
			log::error!("No user_id provided in settings");
			instance.show_alert().await?;
			return Ok(());
		};

		log::debug!(
			"UserVolumeControlButton: user_id={}, mode={}, mute_type={}",
			user_id,
			mode,
			mute_type
		);

		match mode.as_str() {
			"mute" => {
				let current = current_voice_participant(&user_id).await;
				let mute = match mute_type.as_str() {
					"toggle" => Some(!current.as_ref().map(|user| user.mute).unwrap_or(false)),
					"mute" => Some(true),
					"unmute" => Some(false),
					_ => Some(true),
				};
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: None,
						mute,
					},
				)
				.await
			}
			"adjust" => {
				let adjust_value: i32 = settings
					.get("adjust_value")
					.and_then(|v| v.parse().ok())
					.unwrap_or(0);
				let current_volume = current_voice_participant(&user_id)
					.await
					.map(|user| user.volume)
					.unwrap_or(100);
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: Some((current_volume + adjust_value).clamp(0, 200)),
						mute: None,
					},
				)
				.await
			}
			"set" => {
				let set_value: i32 = settings
					.get("set_value")
					.and_then(|v| v.parse().ok())
					.unwrap_or(100);
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: Some(set_value.clamp(0, 200)),
						mute: None,
					},
				)
				.await
			}
			_ => {
				log::error!("Unknown mode: {}", mode);
				instance.show_alert().await?;
				Ok(())
			}
		}
	}

	async fn send_to_plugin(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
		payload: &serde_json::Value,
	) -> OpenActionResult<()> {
		let payload: UserVolumePayload = match serde_json::from_value(payload.clone()) {
			Ok(p) => p,
			Err(e) => {
				log::error!("Failed to parse user volume payload: {}", e);
				return Ok(());
			}
		};

		if payload.action.as_deref() == Some("get_users") {
			log::info!("UserVolumeControlButton received get_users request from PI");
			return send_users_response(instance).await;
		}

		let mut new_settings = HashMap::new();
		if let Some(user_id) = payload.user_id {
			new_settings.insert("user_id".to_string(), user_id);
		}
		if let Some(mode) = payload.mode {
			new_settings.insert("mode".to_string(), mode);
		}
		if let Some(mute_type) = payload.mute_type {
			new_settings.insert("mute_type".to_string(), mute_type);
		}
		if let Some(adjust_value) = payload.adjust_value {
			new_settings.insert("adjust_value".to_string(), adjust_value.to_string());
		}
		if let Some(set_value) = payload.set_value {
			new_settings.insert("set_value".to_string(), set_value.to_string());
		}

		if !new_settings.is_empty() {
			instance.set_settings(&new_settings).await?;
			let merged_settings = merge_instance_settings(instance, &new_settings).await;
			update_user_volume_image(instance, &merged_settings).await?;
		}

		Ok(())
	}
}

pub struct UserVolumeControlDialAction;
#[async_trait]
impl Action for UserVolumeControlDialAction {
	const UUID: ActionUuid = "me.amankhanna.oadiscord.uservolumecontroldial";
	type Settings = HashMap<String, String>;

	async fn will_appear(
		&self,
		instance: &Instance,
		settings: &Self::Settings,
	) -> OpenActionResult<()> {
		remember_instance_settings(instance, settings).await;
		update_user_volume_image(instance, settings).await
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
		update_user_volume_image(instance, settings).await
	}

	async fn property_inspector_did_appear(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
	) -> OpenActionResult<()> {
		log::info!("UserVolumeControlDial property inspector appeared");
		send_users_response(instance).await
	}

	async fn dial_rotate(
		&self,
		instance: &Instance,
		settings: &Self::Settings,
		ticks: i16,
		_pressed: bool,
	) -> OpenActionResult<()> {
		log::debug!(
			"UserVolumeControlDial settings: {:?}, ticks={}",
			settings,
			ticks
		);

		let user_id = settings.get("user_id").cloned();
		let mode = settings
			.get("mode")
			.cloned()
			.unwrap_or_else(|| "adjust".to_string());

		let Some(user_id) = user_id else {
			log::error!("No user_id provided in settings");
			instance.show_alert().await?;
			return Ok(());
		};

		log::debug!("UserVolumeControlDial: user_id={}, mode={}", user_id, mode);

		match mode.as_str() {
			"adjust" => {
				let step = settings
					.get("adjust_value")
					.and_then(|v| v.parse().ok())
					.unwrap_or(5_i32)
					.abs()
					.max(1);
				let current_volume = current_voice_participant(&user_id)
					.await
					.map(|user| user.volume)
					.unwrap_or(100);
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: Some((current_volume + i32::from(ticks) * step).clamp(0, 200)),
						mute: None,
					},
				)
				.await
			}
			"set" => {
				let set_value: i32 = settings
					.get("set_value")
					.and_then(|v| v.parse().ok())
					.unwrap_or(100);
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: Some(set_value.clamp(0, 200)),
						mute: None,
					},
				)
				.await
			}
			_ => Ok(()),
		}
	}

	async fn dial_up(
		&self,
		instance: &Instance,
		settings: &Self::Settings,
	) -> OpenActionResult<()> {
		log::debug!("UserVolumeControlDial press settings: {:?}", settings);

		let user_id = settings.get("user_id").cloned();
		let mode = settings
			.get("mode")
			.cloned()
			.unwrap_or_else(|| "adjust".to_string());

		let Some(user_id) = user_id else {
			log::error!("No user_id provided in settings");
			instance.show_alert().await?;
			return Ok(());
		};

		match mode.as_str() {
			"mute" => {
				let mute_type = settings
					.get("mute_type")
					.cloned()
					.unwrap_or_else(|| "toggle".to_string());
				let current = current_voice_participant(&user_id).await;
				let mute = match mute_type.as_str() {
					"toggle" => Some(!current.as_ref().map(|user| user.mute).unwrap_or(false)),
					"mute" => Some(true),
					"unmute" => Some(false),
					_ => Some(true),
				};
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: None,
						mute,
					},
				)
				.await
			}
			"set" => {
				let set_value: i32 = settings
					.get("set_value")
					.and_then(|v| v.parse().ok())
					.unwrap_or(100);
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: Some(set_value.clamp(0, 200)),
						mute: None,
					},
				)
				.await
			}
			_ => Ok(()),
		}
	}

	async fn send_to_plugin(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
		payload: &serde_json::Value,
	) -> OpenActionResult<()> {
		let payload: UserVolumePayload = match serde_json::from_value(payload.clone()) {
			Ok(p) => p,
			Err(e) => {
				log::error!("Failed to parse user volume payload: {}", e);
				return Ok(());
			}
		};

		if payload.action.as_deref() == Some("get_users") {
			log::info!("UserVolumeControlDial received get_users request from PI");
			return send_users_response(instance).await;
		}

		let mut new_settings = HashMap::new();
		if let Some(user_id) = payload.user_id {
			new_settings.insert("user_id".to_string(), user_id);
		}
		if let Some(mode) = payload.mode {
			new_settings.insert("mode".to_string(), mode);
		}
		if let Some(mute_type) = payload.mute_type {
			new_settings.insert("mute_type".to_string(), mute_type);
		}
		if let Some(adjust_value) = payload.adjust_value {
			new_settings.insert("adjust_value".to_string(), adjust_value.to_string());
		}
		if let Some(set_value) = payload.set_value {
			new_settings.insert("set_value".to_string(), set_value.to_string());
		}

		if !new_settings.is_empty() {
			instance.set_settings(&new_settings).await?;
			let merged_settings = merge_instance_settings(instance, &new_settings).await;
			update_user_volume_image(instance, &merged_settings).await?;
		}

		Ok(())
	}
}

async fn send_users_response(instance: &Instance) -> OpenActionResult<()> {
	let client_available = discord_client().read().await.is_some();
	let channel_id = current_subscribed_voice_channel().await;
	let users = current_voice_participants().await;
	let error = if !client_available {
		Some("Discord client not initialized. Check the plugin connection settings.".to_string())
	} else if channel_id.is_none() {
		Some("Join a Discord voice channel to list participants.".to_string())
	} else {
		None
	};

	log::debug!(
		"Sending users_result to PI: channel_id={}, users={}, error={}",
		channel_id.as_deref().unwrap_or("<none>"),
		users.len(),
		error.as_deref().unwrap_or("<none>")
	);
	let response = UsersResponse {
		action: "users_result".to_string(),
		users,
		error,
	};
	instance
		.send_to_property_inspector(&serde_json::to_string(&response).unwrap())
		.await
}

#[cfg(test)]
mod tests {
	use super::user_volume_image_filename;

	#[test]
	fn image_filename_changes_with_mute_state() {
		let unmuted = user_volume_image_filename("1", "avatar", false);
		let muted = user_volume_image_filename("1", "avatar", true);
		assert_ne!(unmuted, muted);
	}

	#[test]
	fn image_filename_changes_with_avatar_hash() {
		let first = user_volume_image_filename("1", "avatar-a", false);
		let second = user_volume_image_filename("1", "avatar-b", false);
		assert_ne!(first, second);
	}
}
