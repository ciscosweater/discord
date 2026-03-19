use crate::client::current_authenticated_user;

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use openaction::{Action, ActionUuid, Instance, OpenActionResult, async_trait, visible_instances};
use tokio::task;

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

fn stats_image_filename(user_id: &str, avatar_hash: &str) -> String {
	let mut hasher = DefaultHasher::new();
	"user_stats_avatar_only_v1".hash(&mut hasher);
	user_id.hash(&mut hasher);
	avatar_hash.hash(&mut hasher);
	format!("user_stats_{:016x}.png", hasher.finish())
}

fn avatar_cache_filename(user_id: &str, avatar_hash: &str) -> String {
	let mut hasher = DefaultHasher::new();
	"user_stats_avatar_cache_v4".hash(&mut hasher);
	user_id.hash(&mut hasher);
	avatar_hash.hash(&mut hasher);
	format!("user_stats_avatar_{:016x}.png", hasher.finish())
}

async fn cached_avatar_path(user_id: &str, avatar_hash: &str) -> Option<PathBuf> {
	let actions_dir = plugin_actions_dir()?;
	let generated_dir = ensure_generated_dir(&actions_dir)?;
	let avatar_path = generated_dir.join(avatar_cache_filename(user_id, avatar_hash));
	if avatar_path.exists() {
		return Some(avatar_path);
	}

	let url = format!(
		"https://cdn.discordapp.com/avatars/{}/{}.png?size=256",
		user_id, avatar_hash
	);
	let response = match reqwest::get(&url).await {
		Ok(response) => response,
		Err(error) => {
			log::warn!("Failed to fetch avatar for {}: {}", user_id, error);
			return None;
		}
	};
	let response = match response.error_for_status() {
		Ok(response) => response,
		Err(error) => {
			log::warn!(
				"Discord CDN returned an error for avatar {}: {}",
				user_id,
				error
			);
			return None;
		}
	};
	let bytes = match response.bytes().await {
		Ok(bytes) => bytes,
		Err(error) => {
			log::warn!("Failed to read avatar bytes for {}: {}", user_id, error);
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

fn trim_label(input: &str, max_chars: usize) -> String {
	let trimmed = input.trim();
	let mut chars = trimmed.chars();
	let candidate: String = chars.by_ref().take(max_chars).collect();
	if chars.next().is_some() {
		format!("{}...", candidate.trim_end())
	} else {
		candidate
	}
}

fn wrap_title(input: &str, max_line_chars: usize) -> String {
	let trimmed = input.trim();
	if trimmed.is_empty() {
		return String::new();
	}
	if trimmed.chars().count() <= max_line_chars {
		return trimmed.to_string();
	}

	let words: Vec<&str> = trimmed.split_whitespace().collect();
	if words.len() >= 2 {
		let mut first_line = String::new();
		let mut second_line_start = 0usize;

		for (index, word) in words.iter().enumerate() {
			let candidate = if first_line.is_empty() {
				(*word).to_string()
			} else {
				format!("{first_line} {word}")
			};
			if candidate.chars().count() <= max_line_chars || first_line.is_empty() {
				first_line = candidate;
				second_line_start = index + 1;
			} else {
				break;
			}
		}

		if second_line_start < words.len() {
			let second_line = trim_label(&words[second_line_start..].join(" "), max_line_chars);
			return format!("{first_line}\n{second_line}");
		}
	}

	let first_line: String = trimmed.chars().take(max_line_chars).collect();
	let remainder: String = trimmed.chars().skip(max_line_chars).collect();
	format!("{}\n{}", first_line, trim_label(&remainder, max_line_chars))
}

fn compose_user_stats_image(
	avatar_path: &Path,
	user_id: &str,
	avatar_hash: &str,
) -> Option<String> {
	let actions_dir = plugin_actions_dir()?;
	let generated_dir = ensure_generated_dir(&actions_dir)?;
	let filename = stats_image_filename(user_id, avatar_hash);
	let output_path = generated_dir.join(&filename);
	let image_path = format!("actions/generated/{}", filename.trim_end_matches(".png"));
	if output_path.exists() {
		return Some(image_path);
	}

	let blank_svg_path = actions_dir.join("blank.svg");
	let temp_avatar_path = generated_dir.join(format!(
		"user_stats_avatar_fullbleed_{}.png",
		uuid::Uuid::new_v4()
	));

	let avatar_render = Command::new("convert")
		.arg(avatar_path)
		.arg("-resize")
		.arg("144x144^")
		.arg("-gravity")
		.arg("center")
		.arg("-crop")
		.arg("144x144+0+0")
		.arg("+repage")
		.arg(format!("png32:{}", temp_avatar_path.display()))
		.output();
	match avatar_render {
		Ok(output) if output.status.success() => {}
		Ok(output) => {
			log::error!(
				"Failed to render fullbleed stats avatar {}: status={} stderr={}",
				avatar_path.display(),
				output.status,
				String::from_utf8_lossy(&output.stderr)
			);
			return None;
		}
		Err(error) => {
			log::error!(
				"Failed to start convert for fullbleed stats avatar: {}",
				error
			);
			return None;
		}
	}

	let compose = Command::new("convert")
		.arg(&blank_svg_path)
		.arg(&temp_avatar_path)
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
				"Failed to compose user stats image {}: status={} stderr={}",
				output_path.display(),
				output.status,
				String::from_utf8_lossy(&output.stderr)
			);
			None
		}
		Err(error) => {
			log::error!(
				"Failed to start convert for user stats composition: {}",
				error
			);
			None
		}
	};

	let _ = fs::remove_file(&temp_avatar_path);
	result
}

async fn render_user_stats_image() -> Option<String> {
	let user = current_authenticated_user().await?;
	let avatar_hash = user.avatar_hash.as_deref()?.trim().to_string();
	if avatar_hash.is_empty() {
		return None;
	}

	let avatar_path = cached_avatar_path(&user.user_id, &avatar_hash).await?;
	let user_id = user.user_id.clone();
	task::spawn_blocking(move || compose_user_stats_image(&avatar_path, &user_id, &avatar_hash))
		.await
		.ok()
		.flatten()
}

async fn current_user_stats_title() -> String {
	current_authenticated_user()
		.await
		.map(|user| wrap_title(&trim_label(&user.username, 18), 8))
		.unwrap_or_else(|| "User".to_string())
}

async fn update_user_stats_button(instance: &Instance) -> OpenActionResult<()> {
	let title = current_user_stats_title().await;
	let image = render_user_stats_image()
		.await
		.unwrap_or_else(|| "actions/serverStats".to_string());
	instance.set_image(Some(image), None).await?;
	instance.set_title(Some(title), None).await?;
	Ok(())
}

pub async fn refresh_user_stats_instances() {
	for instance in visible_instances(UserStatsAction::UUID).await {
		if let Err(error) = update_user_stats_button(&instance).await {
			log::error!(
				"Failed to refresh user stats image for {}: {}",
				instance.instance_id,
				error
			);
		}
	}
}

pub struct UserStatsAction;
#[async_trait]
impl Action for UserStatsAction {
	const UUID: ActionUuid = "me.amankhanna.oadiscord.userstats";
	type Settings = std::collections::HashMap<String, String>;

	async fn will_appear(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
	) -> OpenActionResult<()> {
		update_user_stats_button(instance).await
	}

	async fn did_receive_settings(
		&self,
		instance: &Instance,
		_settings: &Self::Settings,
	) -> OpenActionResult<()> {
		update_user_stats_button(instance).await
	}
}

#[cfg(test)]
mod tests {
	use super::{stats_image_filename, wrap_title};

	#[test]
	fn image_filename_changes_with_avatar_hash() {
		let first = stats_image_filename("1", "avatar-a");
		let second = stats_image_filename("1", "avatar-b");
		assert_ne!(first, second);
	}

	#[test]
	fn wrap_title_breaks_on_spaces() {
		assert_eq!(wrap_title("ciskao legal", 8), "ciskao\nlegal");
	}
}
