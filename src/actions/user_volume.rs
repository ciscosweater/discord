use crate::client::discord_client;
use crate::rpc_events::{
	current_subscribed_voice_channel, current_voice_participant, current_voice_participants,
};

use std::collections::HashMap;

use discord_ipc_rust::models::send::commands::{SentCommand, SetUserVoiceSettingsArgs};
use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};

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
		}

		Ok(())
	}
}

pub struct UserVolumeControlDialAction;
#[async_trait]
impl Action for UserVolumeControlDialAction {
	const UUID: ActionUuid = "me.amankhanna.oadiscord.uservolumecontroldial";
	type Settings = HashMap<String, String>;

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
