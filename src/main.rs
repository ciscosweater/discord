mod actions;
mod client;
mod oauth;
mod rpc_events;

use actions::*;
use client::schedule_reconnect;

use std::sync::OnceLock;

use openaction::{
	OpenActionResult, async_trait, get_global_settings, global_events, register_action, run,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

// Represents the persisted Discord configuration the Stream Deck host sends us.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct DiscordSettings {
	#[serde(rename = "clientId")]
	pub client_id: String,
	#[serde(rename = "clientSecret")]
	pub client_secret: String,
	#[serde(rename = "accessToken")]
	pub access_token: String,
	pub error: Option<String>,
}

// Global storage for the last-applied settings so every module can read/write them.
pub fn current_settings() -> &'static RwLock<DiscordSettings> {
	static SETTINGS: OnceLock<RwLock<DiscordSettings>> = OnceLock::new();
	SETTINGS.get_or_init(|| RwLock::new(DiscordSettings::default()))
}

// Handles global setting updates pushed from the Stream Deck host.
pub struct GlobalEventHandler;
#[async_trait]
impl global_events::GlobalEventHandler for GlobalEventHandler {
	async fn plugin_ready(&self) -> OpenActionResult<()> {
		get_global_settings().await
	}

	async fn did_receive_global_settings(
		&self,
		event: global_events::DidReceiveGlobalSettingsEvent,
	) -> OpenActionResult<()> {
		let settings: DiscordSettings =
			serde_json::from_value(event.payload.settings).unwrap_or_default();

		log::info!(
			"did_receive_global_settings: client_id present={} client_secret present={} access_token present={}",
			!settings.client_id.is_empty(),
			!settings.client_secret.is_empty(),
			!settings.access_token.is_empty()
		);

		// Only react when the stored settings actually changed so we can avoid reconnect churn.
		let current = current_settings().read().await;
		let settings_changed = current.client_id != settings.client_id
			|| current.client_secret != settings.client_secret
			|| current.access_token != settings.access_token;
		drop(current);

		log::info!("settings_changed={}", settings_changed);

		if settings_changed {
			log::info!("Global settings changed, reinitializing Discord client");

			// Persist the new configuration before attempting to reconnect.
			*current_settings().write().await = settings.clone();

			log::info!("Stored new settings, calling schedule_reconnect");

			schedule_reconnect();
		}

		Ok(())
	}
}

#[tokio::main]
async fn main() -> OpenActionResult<()> {
	{
		use simplelog::*;
		if let Err(error) = TermLogger::init(
			LevelFilter::Debug,
			Config::default(),
			TerminalMode::Stdout,
			ColorChoice::Never,
		) {
			eprintln!("Logger initialization failed: {}", error);
		}
	}

	global_events::set_global_event_handler(&GlobalEventHandler);
	register_action(ToggleMuteAction).await;
	register_action(ToggleDeafenAction).await;
	register_action(PushToMuteAction).await;
	register_action(PushToTalkAction).await;
	register_action(SelectVoiceChannelAction).await;
	register_action(LeaveVoiceChannelAction).await;
	register_action(UserVolumeControlButtonAction).await;
	register_action(UserVolumeControlDialAction).await;
	register_action(PlaySoundboardSoundAction).await;

	run(std::env::args().collect()).await
}
