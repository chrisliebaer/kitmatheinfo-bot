use std::sync::{Arc, Mutex};

use log::{debug, info, trace};
use poise::{Command, CreateReply, Modal};
use serenity::all::{CacheHttp, ChannelId, Color, CreateEmbed, PartialGuild, Ready, RoleId};

use crate::{
	config::{Config, OPhase},
	AppState, Error,
};

pub fn register_commands(commands: &mut Vec<Command<AppState, Error>>) {
	commands.push(ersti());
}

async fn get_role_id(guild: impl Into<PartialGuild>, config: &OPhase) -> Result<RoleId, Error> {
	let guild: PartialGuild = guild.into();
	let Some(role) = guild.role_by_name(&config.role_name) else {
		return Err("Keine Rolle mit dem Namen der O-Phasen-Rolle gefunden".into());
	};
	Ok(role.id)
}

async fn get_channel_id(ctx: poise::ApplicationContext<'_, AppState, Error>, config: &OPhase) -> Result<ChannelId, Error> {
	let guild = ctx
		.guild()
		.ok_or("Dieser Befehl kann nur in einem Server ausgeführt werden.")?
		.clone();
	guild
		.channels(ctx)
		.await?
		.into_iter()
		.find(|(_, channel)| channel.name() == config.channel_name)
		.map(|(id, _)| id)
		.ok_or("Kanal für die O-Phase nicht gefunden".into())
}

pub async fn get_ophase_invite_count(ctx: &poise::serenity_prelude::Context, ready: &Ready, config: &Config) -> Option<u64> {
	if let Some(o_phase_config) = &config.o_phase {
		let mut invite = None;
		trace!("Ready: {:#?}", ready);
		for guild in ready.guilds.iter() {
			let invites = guild
				.id
				.invites(ctx.http())
				.await
				.unwrap_or_else(|e| panic!("Could not get invites for guild {:?}: {e:?}", guild.id));
			trace!("Found invites in guild {:?}: {:?}", guild, invites);
			invite = invites.into_iter().find(|invite| invite.code == o_phase_config.invite_code);
			if invite.is_some() {
				break;
			}
		}
		let invite = invite.expect("Could not find invite for O-Phase code");

		info!("O-Phase invite has {} uses", invite.uses);

		Some(invite.uses)
	} else {
		None
	}
}

pub async fn handle_new_guild_member(
	ctx: &poise::serenity_prelude::Context,
	new_member: &poise::serenity_prelude::Member,
	o_phase_config: &OPhase,
	ophase_invite_uses: &Arc<Mutex<Option<u64>>>,
) -> Result<(), Error> {
	trace!(
		"Checking invite for new member: {} ({})",
		new_member.user.name,
		new_member.user.id
	);

	let guild = new_member.guild_id;
	let guild_invites = guild.invites(&ctx.http()).await?;
	let Some(invite) = guild_invites
		.into_iter()
		.find(|invite| invite.code == o_phase_config.invite_code)
	else {
		return Ok(());
	};
	let new_invite_uses = invite.uses;
	let ophase_invite_uses = {
		let mut count = ophase_invite_uses.lock().unwrap();
		let uses = count.as_mut().unwrap();
		let previous = *uses;
		*uses = new_invite_uses;
		previous
	};

	trace!("Invite uses: new = {}, old = {:?}", new_invite_uses, ophase_invite_uses);

	if new_invite_uses > ophase_invite_uses {
		info!(
			"New O-Phase member through invite: {} ({})",
			new_member.user.name, new_member.user.id
		);
		let role_id = get_role_id(guild.to_partial_guild(ctx.http()).await?, o_phase_config).await?;
		new_member.add_role(ctx.http(), role_id).await?;
	}
	Ok(())
}

/// Für Erstis der kitmatheinfo.de O-Phasengruppe
#[poise::command(slash_command, rename = "ophase")]
async fn ersti(ctx: poise::ApplicationContext<'_, AppState, Error>) -> Result<(), Error> {
	debug!("Executing command: {} ({})", ctx.author().name, ctx.author().id);

	let Some(member) = ctx.author_member().await else {
		return Err("Dieser Befehl kann nicht in DMs ausgeführt werden".into());
	};

	let Some(config) = &ctx.data.config.o_phase else {
		return Err("O-Phase Funktionalität ist nicht konfiguriert".into());
	};

	let guild = ctx
		.guild()
		.ok_or("Dieser Befehl kann nur in einem Server ausgeführt werden.")?
		.clone();

	let role_id = get_role_id(guild, config).await?;
	let channel_id = get_channel_id(ctx, config).await?;

	let Some(response) = PasswordResponse::execute(ctx).await? else {
		debug!("Abgebrochen: {} ({})", ctx.author().name, ctx.author().id);
		return Ok(());
	};

	if response.password.to_lowercase() != config.password.to_lowercase() {
		info!(
			"Falsches Passwort '{}': {} ({})",
			response.password,
			ctx.author().name,
			ctx.author().id
		);

		let reply = CreateReply::default().ephemeral(true).embed(
			CreateEmbed::new()
				.color(Color::from_rgb(255, 99, 71))
				.title("Falsches Gruppen-Passwort")
				.description("Sorry, das ist nicht das korrekte Gruppen-Passwort. Frage bitte noch einmal nach :)"),
		);
		ctx.send(reply).await?;
		return Ok(());
	}

	debug!("Richtiges Passwort: {} ({})", ctx.author().name, ctx.author().id);

	member.add_role(ctx.http(), role_id).await?;

	info!("Nutzer hinzugefügt: {} ({})", ctx.author().name, ctx.author().id);

	let reply = ctx.reply_builder(
		CreateReply::default().reply(true).ephemeral(true).embed(
			CreateEmbed::new()
				.color(Color::from_rgb(25, 177, 241))
				.title("Willkommen in der kitmatheinfo.de O-Phase!")
				.description(format!("Wir sehen uns in <#{}> :)", channel_id)),
		),
	);
	ctx.send(reply).await?;

	Ok(())
}

#[derive(Debug, Modal)]
#[name = "kitmatheinfo.de O-Phase Erstis"]
#[paragraph = "heyy"]
struct PasswordResponse {
	#[name = "Gruppen-Passwort"]
	#[placeholder = "Quack..."]
	#[min_length = 5]
	#[max_length = 40]
	password: String,
}
