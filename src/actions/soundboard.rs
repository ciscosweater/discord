use crate::client::discord_client;

use std::collections::HashMap;

use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
struct SoundboardPayload {
	sound_id: Option<String>,
	guild_id: Option<String>,
	sound_name: Option<String>,
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
