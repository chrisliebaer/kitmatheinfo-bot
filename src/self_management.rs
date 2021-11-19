#[allow(unused_imports)]
use log::{trace, debug, info, warn, error};
use crate::{AppState, Context, Error};
use poise::{
	serenity_prelude::{
		GuildChannel,
	},
	FrameworkBuilder,
};
use poise::serenity::prelude::Mentionable;
use poise::serenity_prelude::{ChannelId, CreateEmbed, GuildId};

pub fn register_commands(builder: FrameworkBuilder<AppState, Error>) -> FrameworkBuilder<AppState, Error> {
	builder.command(channel_dummy(), |f| {
		f.category("Selbstverwaltung")
				.subcommand(create_channel(), |f| f)
				.subcommand(update_channel(), |f| f)
				.subcommand(delete_channel(), |f| f)
	})
}

/// Enthält Befehle für den selbstverwalteten Bereich des Servers.
#[poise::command(slash_command, rename = "channel")]
async fn channel_dummy(_ctx: Context<'_>) -> Result<(), Error> {
	unreachable!() // Upper commands can never be called from discord, all good.
}

/// Erstellt einen neuen Channel.
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

	let guild_id = ctx.guild_id().ok_or("not in guild")?;

	// create channel in category (will fail if in different guild)
	ctx.defer_ephemeral().await?;
	let channel = guild_id.create_channel(ctx.discord(), |c| {
		c.name(name).topic(beschreibung).category(sm.category)
	}).await?;

	// inform user about success
	ctx.send(|m| {
		m.content(format!("Ich hab deinen Channel erstellt: {}", channel.mention()))
	}).await?;

	log_modification(&ctx, "Channel erstellt", None, Some(&channel)).await?;

	sort(&ctx, &guild_id).await?;

	Ok(())
}

/// Modifiziert den angegebenen Channel.
#[poise::command(slash_command, rename = "ändern")]
async fn update_channel(
	ctx: Context<'_>,
	#[description = "Der Name des Channels."]
	channel: GuildChannel,
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
	if channel.guild_id != guild {
		return Err(Error::from("Dieser Channel ist nicht von diesem Server"));
	}

	// check if channel actually belong into self managed category
	if &app.config.self_managment.category != channel.parent_id.ok_or("channel has no parent")?.as_u64() {
		return Err(Error::from("Dieser Channel befindet sich nicht in der richtigen Kategorie und kann nicht gelöscht werden."));
	}

	ctx.defer_ephemeral().await?;
	let after = channel.id.edit(ctx.discord(), |c| {
		c
				.name(name.as_ref().unwrap_or(&channel.name))
				.topic(beschreibung.as_ref().unwrap_or(channel.topic.as_ref().unwrap()))
				.nsfw(nsfw.unwrap_or(channel.nsfw))
	}).await?;

	// inform user about success
	ctx.send(|m| {
		m.content(format!("Ich hab den Channel modifiziert: {}", channel.name()))
	}).await?;
	log_modification(&ctx, "Channel aktualisiert", Some(&channel), Some(&after)).await?;

	// sort channel list to maintain peace and harmony
	sort(&ctx, &guild).await?;

	Ok(())
}

/// Löscht den angegebenen Channel.
#[poise::command(slash_command, rename = "löschen")]
async fn delete_channel(
	ctx: Context<'_>,
	#[description = "Der Channel, den du löschen möchtest."]
	channel: GuildChannel,
) -> Result<(), Error> {
	let app = ctx.data();

	// TODO: duplicated code with update command, fix that
	// check if channel belongs to same guild (prevents deletion from outside guilds)
	let guild = ctx.guild_id().ok_or("Dieser Befehl kann nur in einem Server ausgeführt werden.")?;
	if channel.guild_id != guild {
		return Err(Error::from("Dieser Channel ist nicht von diesem Server"));
	}

	// check if channel actually belong into self managed category
	if &app.config.self_managment.category != channel.parent_id.ok_or("channel has no parent")?.as_u64() {
		return Err(Error::from("Dieser Channel befindet sich nicht in der richtigen Kategorie und kann nicht gelöscht werden."));
	}

	// perform deletion
	ctx.defer_ephemeral().await?;
	channel.delete(ctx.discord()).await?;

	// inform user about success
	ctx.send(|m| {
		m.content(format!("Ich hab den Channel gelöscht: {}", channel.name()))
	}).await?;
	log_modification(&ctx, "Channel gelöscht", Some(&channel), None).await?;

	Ok(())
}

/// Logs modifications in the configured logging channel for transparency and general awareness.
async fn log_modification(
	ctx: &Context<'_>,
	summary: &str,
	before: Option<&GuildChannel>,
	after: Option<&GuildChannel>,
) -> Result<(), Error> {
	// check if logging is enabled
	let log_channel: ChannelId = match &ctx.data().config.self_managment.logging {
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

	if field_set {
		log_channel.send_message(ctx.discord(), |m| {
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
