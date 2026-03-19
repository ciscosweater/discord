use crate::client::discord_client;

use std::collections::HashMap;

use discord_ipc_rust::models::send::commands::{SetUserVoiceSettingsArgs, SentCommand};
use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};

#[derive(Debug, serde::Deserialize)]
struct UserVolumePayload {
	user_id: Option<String>,
	mode: Option<String>,
	mute_type: Option<String>,
	adjust_value: Option<i32>,
	set_value: Option<i32>,
}

async fn update_user_voice_setting(
	instance: &Instance,
	args: SetUserVoiceSettingsArgs,
) -> OpenActionResult<()> {
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

	async fn key_up(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		let user_id = settings.get("user_id").cloned();
		let mode = settings.get("mode").cloned().unwrap_or_else(|| "mute".to_string());
		let mute_type = settings
			.get("mute_type")
			.cloned()
			.unwrap_or_else(|| "toggle".to_string());

		let Some(user_id) = user_id else {
			log::error!("No user_id provided in settings");
			instance.show_alert().await?;
			return Ok(());
		};

		match mode.as_str() {
			"mute" => {
				let mute = match mute_type.as_str() {
					"toggle" => None, // Would need current state - skip for now
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
				// Volume adjustment requires knowing current volume
				// For now, just set to a relative value
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: Some(100 + adjust_value),
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
		}

		Ok(())
	}
}

pub struct UserVolumeControlDialAction;
#[async_trait]
impl Action for UserVolumeControlDialAction {
	const UUID: ActionUuid = "me.amankhanna.oadiscord.uservolumecontroldial";
	type Settings = HashMap<String, String>;

	async fn key_up(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		let user_id = settings.get("user_id").cloned();
		let mode = settings.get("mode").cloned().unwrap_or_else(|| "adjust".to_string());

		let Some(user_id) = user_id else {
			log::error!("No user_id provided in settings");
			instance.show_alert().await?;
			return Ok(());
		};

		match mode.as_str() {
			"adjust" => {
				let adjust_value: i32 = settings
					.get("adjust_value")
					.and_then(|v| v.parse().ok())
					.unwrap_or(0);
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: Some((100 + adjust_value).clamp(0, 200)),
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
				// For dial with mute mode, toggle mute
				update_user_voice_setting(
					instance,
					SetUserVoiceSettingsArgs {
						user_id,
						pan: None,
						volume: None,
						mute: Some(true),
					},
				)
				.await
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
		}

		Ok(())
	}
}