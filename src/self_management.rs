use std::time::{Duration, SystemTime};

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};
use poise::{
	serenity_prelude::{ChannelId, CreateEmbed, GuildChannel, GuildId, Mention, Mentionable, User, UserId},
	Command, CreateReply,
};
use serde::{Deserialize, Serialize};
use serenity::{
	all::{CreateChannel, CreateMessage},
	builder::EditChannel,
};

use crate::{config::SelfManagement, AppState, Context, Error};

const CHANNEL_EDIT_TIMEOUT: Duration = Duration::from_secs(10);

pub fn register_commands(commands: &mut Vec<Command<AppState, Error>>) {
	commands.push(channel_dummy());
}

/// Enthält Befehle für den selbstverwalteten Bereich des Servers.
#[poise::command(
	slash_command,
	rename = "kanal",
	subcommands("create_channel", "update_channel", "delete_channel", "claim_channel")
)]
async fn channel_dummy(_ctx: Context<'_>) -> Result<(), Error> {
	unreachable!() // Upper commands can never be called from discord, all good.
}

/// Erstellt einen neuen Kanal.
#[poise::command(slash_command, rename = "erstellen")]
async fn create_channel(
	ctx: Context<'_>,
	#[description = "Der Name des Channels."] name: String,
	#[description = "Wofür ist dieser Channel?"] beschreibung: String,
) -> Result<(), Error> {
	let app = ctx.data();
	let sm = &app.config.self_managment;

	let guild_id = ctx
		.guild_id()
		.ok_or("Dieser Befehl kann nur auf einem Server ausgeführt werden.")?;

	// check how long user has been on the server
	let joined_at = ctx
		.author_member()
		.await
		.and_then(|m| m.joined_at)
		.ok_or("Ich konnte dein Mitgliedsalter leider nicht abfragen.")?;
	let joined_at = joined_at.timestamp();
	let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() as i64;

	if now - joined_at < sm.join_age_limit {
		return Err(Error::from(
			"Du bist noch nicht lange genug auf dem Server, um einen Kanal zu erstellen.",
		));
	}

	// get current channels owned by user
	let owned_channels = get_user_channel(&ctx).await?;
	trace!(
		"User owns {} channels: {:?}",
		owned_channels.len(),
		owned_channels.iter().map(|c| c.name.clone()).collect::<Vec<_>>()
	);

	// check if user is allowed to create more channels
	if owned_channels.len() >= sm.limit as usize {
		return Err(Error::from(format!("Du darfst nur maximal {} Kanäle besitzen.", sm.limit)));
	}

	// create channel in category (will fail if in different guild)
	ctx.defer_ephemeral().await?;
	let channel = guild_id
		.create_channel(
			ctx,
			CreateChannel::new(name)
				.topic(inject_ownership(&beschreibung, ctx.author(), app))
				.category(sm.category),
		)
		.await?;

	// recent undocumented change discards any newlines in the topic during creation, so we have to set it again via edit
	// call (costs another api call, great)
	channel
		.id
		.edit(
			ctx,
			EditChannel::default().topic(inject_ownership(&beschreibung, ctx.author(), app)),
		)
		.await?;

	// inform user about success
	ctx
		.send(CreateReply::default().content(format!("Ich hab deinen Kanal erstellt: {}", channel.mention())))
		.await?;

	log_both(&ctx, "Kanal erstellt", None, Some(&channel)).await?;

	sort(&ctx, &guild_id).await?;

	Ok(())
}

/// Modifiziert den angegebenen Kanal.
#[poise::command(slash_command, rename = "ändern")]
async fn update_channel(
	ctx: Context<'_>,
	#[description = "Der Name des Channels."] kanal: GuildChannel,
	#[description = "Eine neuer Name."] name: Option<String>,
	#[description = "Eine neue Beschreibung."] beschreibung: Option<String>,
	#[description = "Setzt den NSFW Status."] nsfw: Option<bool>,
) -> Result<(), Error> {
	let guild = precheck_and_unwrap(ctx, &kanal)?;
	let app = ctx.data();

	// enforce ownership, if enabled
	if !can_edit_channel(&ctx.author().id, &kanal, &app.config.self_managment) {
		return Err(Error::from("Du darfst diesen Kanal nicht bearbeiten."));
	}
	ctx.defer_ephemeral().await?;

	let after = {
		let mut edit_channel = EditChannel::default()
			.name(name.as_ref().unwrap_or(&kanal.name))
			.nsfw(nsfw.unwrap_or(kanal.nsfw));
		let topic = beschreibung.as_ref();
		if let Some(topic) = topic {
			edit_channel = edit_channel.topic(inject_ownership(topic, ctx.author(), app));
		}

		kanal.id.edit(ctx, edit_channel)
	};

	// channel edits have absolute bonkers rate limits, so to prevent a lot of work to stack up, we use aggressive timeouts
	let after = tokio::time::timeout(CHANNEL_EDIT_TIMEOUT, after).await??;

	// inform user about success
	ctx
		.send(CreateReply::default().content(format!("Ich hab den Channel modifiziert: {}", kanal.name())))
		.await?;
	log_both(&ctx, "Channel aktualisiert", Some(&kanal), Some(&after)).await?;

	// sort channel list to maintain peace and harmony
	sort(&ctx, &guild).await?;

	Ok(())
}

/// Beansprucht den angegebenen Kanal.
#[poise::command(slash_command, rename = "aneignen")]
async fn claim_channel(ctx: Context<'_>, #[description = "Der Name des Channels."] kanal: GuildChannel) -> Result<(), Error> {
	let _guild = precheck_and_unwrap(ctx, &kanal)?;
	let app = ctx.data();

	if !app.config.self_managment.claiming || !app.config.self_managment.ownership {
		return Err(Error::from("Kanalübernahmen sind deaktiviert."));
	}

	// enforce ownership, if enabled
	if !can_edit_channel(&ctx.author().id, &kanal, &app.config.self_managment) {
		return Err(Error::from("Du darfst diesen Kanal nicht übernehmen."));
	}
	ctx.defer_ephemeral().await?;

	// check if channel contains owner information
	let meta = ChannelMeta::from_channel(&kanal);
	let topic = match meta {
		Some(_) => remove_meta(kanal.topic.as_ref().unwrap()),
		None => kanal.topic.as_deref().unwrap_or("").to_string(),
	};

	let after = kanal
		.id
		.edit(ctx, EditChannel::default().topic(inject_ownership(&topic, ctx.author(), app)));

	// channel edits have absolute bonkers rate limits, so to prevent a lot of work to stack up, we use aggressives timeouts
	tokio::time::timeout(CHANNEL_EDIT_TIMEOUT, after).await??;

	// inform user about success
	ctx
		.send(CreateReply::default().content(format!("Du bist nun der neue Besitzer von: {}", kanal.name())))
		.await?;

	Ok(())
}

/// Löscht den angegebenen Kanal.
#[poise::command(slash_command, rename = "löschen")]
async fn delete_channel(
	ctx: Context<'_>,
	#[description = "Der Channel, den du löschen möchtest."] kanal: GuildChannel,
) -> Result<(), Error> {
	let _guild = precheck_and_unwrap(ctx, &kanal)?;
	let app = ctx.data();

	// enforce ownership, if enabled
	if !can_edit_channel(&ctx.author().id, &kanal, &app.config.self_managment) {
		return Err(Error::from("Du darfst diesen Kanal nicht löschen."));
	}

	// perform deletion
	ctx.defer_ephemeral().await?;
	kanal.delete(ctx).await?;

	// inform user about success (only possible if command was not issued from the very same channel)
	if ctx.channel_id() != kanal.id {
		ctx
			.send(CreateReply::default().content(format!("Ich hab den Kanal gelöscht: {}", kanal.name())))
			.await?;
	}

	log_both(&ctx, "Kanal gelöscht", Some(&kanal), None).await?;

	Ok(())
}

async fn log_both(
	ctx: &Context<'_>,
	summary: &str,
	before: Option<&GuildChannel>,
	after: Option<&GuildChannel>,
) -> Result<(), Error> {
	let cfg = &ctx.data().config.self_managment;
	let user = ctx.author();

	// logging for everyone without user
	log_modification(ctx, &cfg.logging.map(|id| ChannelId::new(id)), summary, before, after, None).await?;

	// internal logging with executing user
	log_modification(
		ctx,
		&cfg.logging_detailed.map(|id| ChannelId::new(id)),
		summary,
		before,
		after,
		Some(user),
	)
	.await?;

	Ok(())
}

/// Logs modifications in the configured logging channel for transparency and general awareness.
async fn log_modification(
	ctx: &Context<'_>,
	channel_id: &Option<ChannelId>,
	summary: &str,
	before: Option<&GuildChannel>,
	after: Option<&GuildChannel>,
	user: Option<&User>,
) -> Result<(), Error> {
	// check if logging is enabled
	let channel_id: ChannelId = match channel_id {
		None => return Ok(()), // do nothing and return
		Some(channel_id) => channel_id.to_owned(),
	};

	let mut e = CreateEmbed::default();

	// track if we actually ever set any field
	let mut field_set = false;

	e = e.title(summary);
	match before {
		None => match after {
			// nothing, caller is stupid
			None => unreachable!("There never was any channel, who dared waking me up?"),

			// new channel
			Some(after) => {
				e = e
					.field("Name", &after.name, true)
					.field("Beschreibung", after.topic.as_ref().unwrap(), true);
				field_set = true;
			},
		},
		Some(before) => match after {
			// channel delete
			None => {
				e = e
					.field("Name", &before.name, true)
					.field("Beschreibung", before.topic.as_ref().unwrap(), true);
				field_set = true;
			},

			// channel was modified
			Some(after) => {
				// check if name changed
				if before.name != after.name {
					e = e.field("Name", format!("- `{}`\n\n+ `{}`", before.name, after.name), true);
					field_set = true;
				}

				// check if description changed
				if before.topic != after.topic {
					e = e.field(
						"Beschreibung",
						format!(
							"- `{}`\n\n+ `{}`",
							before.topic.as_deref().unwrap_or(""),
							after.topic.as_deref().unwrap_or("")
						),
						true,
					);
					field_set = true;
				}

				// check if nsfw flag changed
				if before.topic != after.topic {
					e = e.field("NSFW", if after.nsfw { "ja" } else { "nein" }, true);
					field_set = true;
				}
			},
		},
	};

	if let Some(user) = user {
		e = e.field("Nutzer", format!("{} ({})", user.name, user.id), false);
	}

	if field_set {
		channel_id.send_message(ctx, CreateMessage::default().embed(e)).await?;
	}

	Ok(())
}

async fn get_user_channel(ctx: &Context<'_>) -> Result<Vec<GuildChannel>, Error> {
	let guild = ctx
		.guild()
		.ok_or("Dieser Befehl kann nur in einem Server ausgeführt werden.")?
		.clone();
	let user = ctx.author();
	let sm = &ctx.data().config.self_managment;
	let channels = guild.channels(ctx).await?;

	// keep only channels that are in the category and have ownership meta
	let channels = channels
		.into_values()
		.filter(|c| match c.parent_id {
			Some(id) => id == sm.category,
			None => false,
		})
		.collect::<Vec<_>>();
	trace!("found {} channels in self_management category", channels.len());

	// filter channels by ownership
	let channels = channels
		.into_iter()
		.filter_map(|c| match ChannelMeta::from_channel(&c) {
			Some(meta) => {
				if meta.owner == user.id {
					Some(c)
				} else {
					None
				}
			},
			None => None,
		})
		.collect::<Vec<_>>();

	Ok(channels)
}

fn inject_ownership(topic: &str, user: &User, app: &AppState) -> String {
	if !app.config.self_managment.ownership {
		return topic.to_string();
	}

	let meta = ChannelMeta { owner: user.id };
	let json = serde_json::to_string(&meta).expect("failed to serialize ownership meta");
	format!("{}\n\n{}", topic, json)
}

/// Perform some sanity checks and unwrap the guild object
fn precheck_and_unwrap(ctx: Context<'_>, channel: &GuildChannel) -> Result<GuildId, Error> {
	let app = ctx.data();

	// check if channel belongs to same guild (prevents deletion from outside guilds)
	let guild = ctx
		.guild_id()
		.ok_or("Dieser Befehl kann nur in einem Server ausgeführt werden.")?;
	if channel.guild_id != guild {
		return Err(Error::from("Dieser Channel ist nicht von diesem Server"));
	}

	// check if channel actually belong into self-managed category
	let channel_category_id = channel
		.parent_id
		.ok_or("Dieser Kanal befindet sich nicht unterhalb einer Kategorie.")?;
	if app.config.self_managment.category != u64::from(channel_category_id) {
		return Err(Error::from(
			"Dieser Channel befindet sich nicht in der richtigen Kategorie und kann nicht gelöscht werden.",
		));
	}

	Ok(guild)
}

/// Checks if the user is allowed to edit the channel.
fn can_edit_channel(user: &UserId, channel: &GuildChannel, config: &SelfManagement) -> bool {
	// always allow edit if ownership is disabled
	if !config.ownership {
		return true;
	}

	// check abadonment state
	let is_abandoned = channel
		.last_message_id
		.map(|id| {
			let created = id.created_at();
			let now = poise::serenity_prelude::Timestamp::now();
			let diff = now.timestamp() - created.timestamp();
			diff as u64 > config.abandon_after
		})
		.unwrap_or(false);

	// allow if user is owner or there is no meta
	let meta = ChannelMeta::from_channel(channel);
	if let Some(meta) = meta {
		if meta.owner == *user {
			return true;
		}
	} else {
		// if there is no meta, channel is always considered free for all
		return true;
	}

	// otherwise channel needs to be abandoned to be editable
	if is_abandoned {
		return true;
	}

	false
}

fn remove_meta(str: &str) -> String {
	let mut lines = str.trim().lines().collect::<Vec<_>>();
	lines.truncate(lines.len() - 1);
	lines.join("\n").trim().to_string()
}

async fn sort(ctx: &Context<'_>, guild: &GuildId) -> Result<(), Error> {
	let app = ctx.data();
	let category_id = ChannelId::new(app.config.self_managment.category);

	let channels = guild.channels(ctx).await?;
	let mut category_channels = channels
		.iter()
		// remove all channels without parent
		.filter_map(|(_, channel)| channel.parent_id.map(|parent_id| (parent_id, channel)))
		// remove channels from different categories
		.filter(|(parent_id, _)| category_id == *parent_id)
		// drop parent information
		.map(|(_, channel)| channel)
		.collect::<Vec<_>>();

	category_channels.sort_by(|x, y| x.name.cmp(&y.name));

	guild
		.reorder_channels(
			ctx,
			category_channels
				.into_iter()
				.enumerate()
				.map(|(idx, channel)| (channel.id, idx as u64)),
		)
		.await?;

	Ok(())
}

/// Tracks channel meta data like ownership.
#[derive(Serialize, Deserialize, Debug)]
struct ChannelMeta {
	#[serde(with = "channel_meta_serde")]
	owner: UserId,
}

impl ChannelMeta {
	fn from_channel(channel: &GuildChannel) -> Option<Self> {
		channel
			.topic
			.as_ref()
			.and_then(|topic| topic.trim().lines().last().map(|last| last.to_string()))
			.and_then(|line: String| serde_json::from_str::<Self>(&line).ok())
	}
}

mod channel_meta_serde {
	use std::str::FromStr;

	use serde::{Deserializer, Serializer};

	use super::*;

	pub fn serialize<S>(user_id: &UserId, s: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		s.serialize_str(&user_id.mention().to_string())
	}

	pub fn deserialize<'de, D>(d: D) -> Result<UserId, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s: &str = Deserialize::deserialize(d)?;
		let mention = Mention::from_str(s).map_err(|err| serde::de::Error::custom(format!("Failed to parse mention: {}", err)))?;

		let user_id = match mention {
			Mention::User(user_id) => Ok(user_id),
			_ => Err(serde::de::Error::custom("Expected a user id")),
		}?;

		Ok(user_id)
	}
}
