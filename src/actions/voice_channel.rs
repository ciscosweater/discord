use crate::actions::GuildInfo;
use crate::client::discord_client;
use crate::rpc_events::{
	available_soundboard_guilds, guild_request_error, register_pending_voice_channel_request,
	voice_channel_request_error, voice_channels,
};

use std::collections::HashMap;
use std::sync::OnceLock;

use discord_ipc_rust::models::send::commands::{
	GetChannelsArgs, SelectVoiceChannelArgs, SentCommand,
};
use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};

#[derive(Debug, serde::Deserialize)]
struct VoiceChannelPayload {
	action: Option<String>,
	guild_id: Option<String>,
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

async fn update_join_voice_channel_button(
	instance: &Instance,
	settings: &HashMap<String, String>,
) -> OpenActionResult<()> {
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
	use super::channel_title_enabled;
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
}
