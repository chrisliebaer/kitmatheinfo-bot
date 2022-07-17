use std::time::Duration;
#[allow(unused_imports)]
use log::{trace, debug, info, warn, error};
use crate::{AppState, Context, Error};
use poise::{serenity_prelude::{
	GuildChannel,
}, Command};
use poise::serenity::prelude::Mentionable;
use poise::serenity_prelude::{ChannelId, CreateEmbed, GuildId, User};

const CHANNEL_EDIT_TIMEOUT: Duration = Duration::from_secs(10);

pub fn register_commands(commands: &mut Vec<Command<AppState, Error>>) {
	commands.push(channel_dummy());
}

/// Enthält Befehle für den selbstverwalteten Bereich des Servers.
#[poise::command(slash_command, rename = "kanal", subcommands("create_channel", "update_channel", "delete_channel"))]
async fn channel_dummy(_ctx: Context<'_>) -> Result<(), Error> {
	unreachable!() // Upper commands can never be called from discord, all good.
}

/// Erstellt einen neuen Kanal.
#[poise::command(slash_command, rename = "erstellen")]
async fn create_channel(
	ctx: Context<'_>,
	#[description = "Der Name des Channels."]
	name: String,
	#[description = "Wofür ist dieser Channel?"]
	beschreibung: String,
) -> Result<(), Error> {
	let app = ctx.data();
	let sm = &app.config.self_managment;

	let guild_id = ctx.guild_id().ok_or("Dieser Befehl kann nur auf einem Server ausgeführt werden.")?;

	// create channel in category (will fail if in different guild)
	ctx.defer_ephemeral().await?;
	let channel = guild_id.create_channel(ctx.discord(), |c| {
		c.name(name).topic(beschreibung).category(sm.category)
	}).await?;

	// inform user about success
	ctx.send(|m| {
		m.content(format!("Ich hab deinen Kanal erstellt: {}", channel.mention()))
	}).await?;

	log_both(&ctx, "Kanal erstellt", None, Some(&channel)).await?;

	sort(&ctx, &guild_id).await?;

	Ok(())
}

/// Modifiziert den angegebenen Kanal.
#[poise::command(slash_command, rename = "ändern")]
async fn update_channel(
	ctx: Context<'_>,
	#[description = "Der Name des Channels."]
	kanal: GuildChannel,
	#[description = "Eine neuer Name."]
	name: Option<String>,
	#[description = "Eine neue Beschreibung."]
	beschreibung: Option<String>,
	#[description = "Setzt den NSFW Status."]
	nsfw: Option<bool>,
) -> Result<(), Error> {
	let app = ctx.data();

	// TODO: duplicated code with delete command, fix that
	// check if channel belongs to same guild (prevents deletion from outside guilds)
	let guild = ctx.guild_id().ok_or("Dieser Befehl kann nur in einem Server ausgeführt werden.")?;
	if kanal.guild_id != guild {
		return Err(Error::from("Dieser Channel ist nicht von diesem Server"));
	}

	// check if channel actually belong into self managed category
	if &app.config.self_managment.category != kanal.parent_id.ok_or("Dieser Kanal befindet sich nicht unterhalb einer Kategorie.")?.as_u64() {
		return Err(Error::from("Dieser Channel befindet sich nicht in der richtigen Kategorie und kann nicht gelöscht werden."));
	}

	ctx.defer_ephemeral().await?;
	let after = kanal.id.edit(ctx.discord(), |c| {
		c
				.name(name.as_ref().unwrap_or(&kanal.name))
				.nsfw(nsfw.unwrap_or(kanal.nsfw));

		let topic = beschreibung.as_ref().or(kanal.topic.as_ref());
		if let Some(topic) = topic {
			c.topic(topic);
		}
		c
	});

	// channel edits have absolute bonkers rate limits, so to prevent a lot of work to stack up, we use aggressives timeouts
	let after = tokio::time::timeout(CHANNEL_EDIT_TIMEOUT, after).await??;

	// inform user about success
	ctx.send(|m| {
		m.content(format!("Ich hab den Channel modifiziert: {}", kanal.name()))
	}).await?;
	log_both(&ctx, "Channel aktualisiert", Some(&kanal), Some(&after)).await?;

	// sort channel list to maintain peace and harmony
	sort(&ctx, &guild).await?;

	Ok(())
}

/// Löscht den angegebenen Kanal.
#[poise::command(slash_command, rename = "löschen")]
async fn delete_channel(
	ctx: Context<'_>,
	#[description = "Der Channel, den du löschen möchtest."]
	kanal: GuildChannel,
) -> Result<(), Error> {
	let app = ctx.data();

	// TODO: duplicated code with update command, fix that
	// check if channel belongs to same guild (prevents deletion from outside guilds)
	let guild = ctx.guild_id().ok_or("Dieser Befehl kann nur in einem Server ausgeführt werden.")?;
	if kanal.guild_id != guild {
		return Err(Error::from("Dieser Channel ist nicht von diesem Server."));
	}

	// check if channel actually belong into self managed category
	if &app.config.self_managment.category != kanal.parent_id.ok_or("Dieser Kanal befindet sich nicht unterhalb einer Kategorie.")?.as_u64() {
		return Err(Error::from("Dieser Kanal befindet sich nicht in der richtigen Kategorie und kann nicht gelöscht werden."));
	}

	// perform deletion
	ctx.defer_ephemeral().await?;
	kanal.delete(ctx.discord()).await?;

	// inform user about success
	ctx.send(|m| {
		m.content(format!("Ich hab den Kanal gelöscht: {}", kanal.name()))
	}).await?;

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
	log_modification(ctx, &cfg.logging.map(|id|ChannelId(id)), summary, before, after, None).await?;

	// internal logging with executing user
	log_modification(ctx, &cfg.logging_detailed.map(|id|ChannelId(id)), summary, before, after, Some(user)).await?;

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
		Some(channel_id) => channel_id.to_owned()
	}.into();

	let mut e = CreateEmbed::default();

	// track if we actually ever set any field
	let mut field_set = false;

	e.title(summary);
	match before {
		None => match after {

			// nothing, caller is stupid
			None => unreachable!("There never was any channel, who dared waking me up?"),

			// new channel
			Some(after) => {
				e.field("Name", &after.name, true).field("Beschreibung", after.topic.as_ref().unwrap(), true);
				field_set = true;
			},
		}
		Some(before) =>
			match after {

				// channel delete
				None => {
					e.field("Name", &before.name, true).field("Beschreibung", before.topic.as_ref().unwrap(), true);
					field_set = true;
				},

				// channel was modified
				Some(after) => {

					// check if name changed
					if before.name != after.name {
						e.field("Name", format!("- `{}`\n\n+ `{}`", before.name, after.name), true);
						field_set = true;
					}

					// check if description changed
					if before.topic != after.topic {
						e.field("Beschreibung", format!("- `{}`\n\n+ `{}`",
																						before.topic.as_deref().unwrap_or(""),
																						after.topic.as_deref().unwrap_or("")
						), true);
						field_set = true;
					}

					// check if nsfw flag changed
					if before.topic != after.topic {
						e.field("NSFW", if after.nsfw { "ja" } else { "nein" }, true);
						field_set = true;
					}
				}
			},
	};

	if let Some(user) = user {
		e.field("Nutzer", user, false);
	}

	if field_set {
		channel_id.send_message(ctx.discord(), |m| {
			m.set_embed(e);
			m
		}).await?;
	}

	Ok(())
}

async fn sort(
	ctx: &Context<'_>,
	guild: &GuildId,
) -> Result<(), Error> {
	let app = ctx.data();
	let category_id = ChannelId(app.config.self_managment.category);

	let channels = guild.channels(ctx.discord()).await?;
	let mut category_channels = channels.iter()

			// remove all channels without parent
			.filter_map(|(_, channel)|channel.parent_id.map(|parent_id| (parent_id, channel)))

			// remove channels from different categories
			.filter(|(parent_id, _)| category_id.0 == parent_id.0)

			// drop parent information
			.map(|(_, channel)| channel)
			.collect::<Vec<_>>();

	category_channels.sort_by(|x, y| x.name.cmp(&y.name));

	guild.reorder_channels(ctx.discord(), category_channels.into_iter()
			.enumerate()
			.map(|(idx, channel)| (channel.id, idx as u64))
	).await?;

	Ok(())
}
