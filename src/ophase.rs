use log::{
	debug,
	info,
};
use poise::{
	Command,
	CreateReply,
	Modal,
};
use serenity::all::{
	ChannelId,
	Color,
	CreateEmbed,
	RoleId,
};

use crate::{
	config::OPhase,
	AppState,
	Error,
};

pub fn register_commands(commands: &mut Vec<Command<AppState, Error>>) {
	commands.push(ersti());
}

async fn get_role_id(ctx: poise::ApplicationContext<'_, AppState, Error>, config: &OPhase) -> Result<RoleId, Error> {
	let guild = ctx
		.guild()
		.ok_or("Dieser Befehl kann nur in einem Server ausgeführt werden.")?
		.clone();

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
	let role_id = get_role_id(ctx, config).await?;
	let channel_id = get_channel_id(ctx, config).await?;

	let Some(response) = PasswordResponse::execute(ctx).await? else {
		debug!("Abgebrochen: {} ({})", ctx.author().name, ctx.author().id);
		return Ok(());
	};

	if response.password != config.password {
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
