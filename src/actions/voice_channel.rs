use crate::client::discord_client;

use std::collections::HashMap;

use discord_ipc_rust::models::send::commands::{SelectVoiceChannelArgs, SentCommand};
use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait};

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
		Ok(_) => {
			instance.show_ok().await?;
		}
		Err(e) => {
			log::error!("Failed to select voice channel: {}", e);
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

	async fn key_up(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		let channel_id = settings.get("channel_id").cloned();

		if let Some(id) = channel_id {
			update_voice_channel(
				instance,
				SelectVoiceChannelArgs {
					channel_id: Some(id),
					force: Some(true),
					timeout: None,
					navigate: None,
				},
			)
			.await
		} else {
			log::error!("No channel_id provided in settings");
			instance.show_alert().await?;
			Ok(())
		}
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
