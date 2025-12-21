use std::collections::HashSet;

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};
use poise::{
	serenity_prelude::{
		ButtonStyle, ChannelType, CreateActionRow, CreateSelectMenu, CreateSelectMenuOption, GuildChannel, Message, RoleId,
	},
	Command, CreateReply,
};
use serenity::{
	all::{
		ComponentInteraction, ComponentInteractionDataKind, CreateButton, CreateInteractionResponse,
		CreateInteractionResponseMessage, CreateMessage, CreateSelectMenuKind, EditMessage,
	},
	builder::{CreateAllowedMentions, EditInteractionResponse},
};

use crate::{AppState, Context, Error};

pub fn register_commands(commands: &mut Vec<Command<AppState, Error>>) {
	commands.push(post_welcome_message());
	commands.push(update_welcome_message());
}

fn get_toc_buttons(app: &AppState) -> Vec<CreateActionRow> {
	// adds buttons for toc records
	let mut buttons = Vec::new();
	buttons.push(
		CreateButton::new("assignments")
			.label(&app.config.self_assignments.label)
			.emoji(app.config.self_assignments.icon.clone())
			.style(ButtonStyle::Success),
	);
	for entry in &app.config.toc {
		buttons.push(
			CreateButton::new(format!("toc:{}", entry.file.filename))
				.label(&entry.label)
				.emoji(entry.icon.to_owned())
				.style(ButtonStyle::Primary),
		);
	}
	vec![CreateActionRow::Buttons(buttons)]
}

/// Aktualisiert die verlinkte Nachricht auf die aktuelle Begrüßung.
#[poise::command(prefix_command, rename = "rewelcome", required_permissions = "MANAGE_GUILD")]
async fn update_welcome_message(
	ctx: Context<'_>,
	#[description = "Die Nachricht, welche aktualisiert werden soll."] mut message: Message,
) -> Result<(), Error> {
	let app = ctx.data();

	let guild = ctx.guild_id().ok_or("not in guild")?;
	let channels = guild.channels(&ctx).await?;

	if !channels.contains_key(&message.channel_id) {
		return Err(Error::from("target message was not posted in this guild"));
	}

	message
		.edit(
			&ctx,
			EditMessage::default()
				.content(app.config.welcome.to_string())
				.suppress_embeds(true)
				.components(get_toc_buttons(app)),
		)
		.await?;

	ctx
		.send(
			CreateReply::default()
				.content("Nachricht erfolgreich aktualisiert.")
				.ephemeral(true),
		)
		.await?;

	Ok(())
}

/// Erstellt die Begrüßungsnachricht im angegebenen Channel.
#[poise::command(prefix_command, rename = "welcome", required_permissions = "MANAGE_GUILD")]
async fn post_welcome_message(
	ctx: Context<'_>,
	#[description = "Der Channel in dem Nachricht erstellt werden soll."] channel: GuildChannel,
) -> Result<(), Error> {
	let app = ctx.data();

	if ctx.guild_id().ok_or("not in guild")? != channel.guild_id {
		return Err(Error::from("current guild differs from guild of target channel"));
	}

	// TODO: this can be done by using the "channel_types" field, but is not supported by poise
	if channel.kind != ChannelType::Text {
		return Err(Error::from("not a text channel"));
	}

	channel
		.send_message(
			&ctx,
			CreateMessage::default()
				.content(app.config.welcome.to_string())
				.components(get_toc_buttons(app)),
		)
		.await?;

	ctx
		.send(
			CreateReply::default()
				.content("Nachricht erfolgreich erstellt")
				.ephemeral(true),
		)
		.await?;

	Ok(())
}

pub async fn handle_assign_click<'a>(
	ctx: &'a poise::serenity_prelude::Context,
	app: &'a AppState,
	interaction: &'a ComponentInteraction,
) -> Result<(), Error> {
	let data = &interaction.data;
	let ComponentInteractionDataKind::StringSelect {
		values: selected_role_strings,
	} = &data.kind
	else {
		panic!("invalid select type")
	};

	// toc buttons are identified as `toc:$file`
	let custom_id = data.custom_id.as_str();
	let id = custom_id
		.splitn(2, ":")
		.last()
		.ok_or(format!("Unknown format in toc custom_id: {}", custom_id))?;

	let assignment = app.config.assignments.get(id).ok_or(format!("Unknown assignment: {}", id))?;
	let member = interaction
		.member
		.as_ref()
		.ok_or("not executed in guild, no way to assign roles")?;

	// calculate ids of all roles in assignment
	let all_roles = &assignment.roles.iter().map(|a| a.role).collect::<HashSet<_>>();

	// menu options contain corresponding roles, so extract them
	let selected = selected_role_strings
		.iter()
		.map(|x| x.parse::<u64>())
		.collect::<Result<HashSet<u64>, _>>()?;

	// current roles of user, important since discord will reject modifications with preexisting role assignments
	let current = member
		.roles
		.iter()
		.map(|x| u64::from(*x).to_owned())
		.collect::<HashSet<u64>>();

	// remove all roles which are not selected, but only if user currently has them
	let removed_roles = &(all_roles - &selected) & &current;

	// add all selected roles but only if user does not already have them
	let new_roles = &selected - &current;

	// convert these sets to vec since we need slices for api calls
	// TODO: this is probably a much nicer way to accomplish the same thing
	let new_roles = new_roles.into_iter().map(|x| x.to_owned().into()).collect::<Vec<RoleId>>();
	let removed_roles = removed_roles
		.into_iter()
		.map(|x| x.to_owned().into())
		.collect::<Vec<RoleId>>();

	// we need to react to the interaction since role update could cause too much delay
	interaction
		.create_response(
			ctx,
			CreateInteractionResponse::Message(
				CreateInteractionResponseMessage::new()
					.content("Ich aktualisiere deine Rollen...".to_string())
					.flags(poise::serenity_prelude::InteractionResponseFlags::EPHEMERAL),
			),
		)
		.await?;

	// apply role modifications (remove comes first to prevent permission escalation between updates)
	let member = member.clone();
	member.remove_roles(ctx, removed_roles.as_slice()).await.unwrap();
	member.add_roles(ctx, new_roles.as_slice()).await.unwrap();

	// update initial response and notify user about success
	{
		let new_roles = new_roles
			.into_iter()
			.map(|role| format!("<@&{role}>"))
			.collect::<Vec<_>>()
			.join(", ");
		let removed_roles = removed_roles
			.into_iter()
			.map(|role| format!("<@&{role}>"))
			.collect::<Vec<_>>()
			.join(", ");
		let content = format!(
			r#"
**Rollen erfolgreich angepasst**
Neue Rollen: {}

Entfernte Rollen: {}
				"#,
			new_roles, removed_roles
		);

		interaction
			.edit_response(
				ctx,
				EditInteractionResponse::new()
					.allowed_mentions(CreateAllowedMentions::default())
					.content(content),
			)
			.await?;
	}

	Ok(())
}

pub async fn handle_toc_click<'a>(
	ctx: &'a poise::serenity_prelude::Context,
	app: &'a AppState,
	interaction: &'a ComponentInteraction,
) -> Result<(), Error> {
	let data = &interaction.data;

	// assign buttons are identified as `assign:$file`
	let custom_id = data.custom_id.as_str();
	let file = custom_id
		.splitn(2, ":")
		.last()
		.ok_or(format!("Unknown format in assign custom_id: {}", custom_id))?;

	let entry = app
		.config
		.toc
		.iter()
		.find(|f| f.file.filename == file)
		.ok_or(format!("Unknown toc file: {}", file))?;

	interaction
		.create_response(
			ctx,
			CreateInteractionResponse::Message(
				CreateInteractionResponseMessage::new()
					.content(entry.file.content.to_string())
					.flags(poise::serenity_prelude::InteractionResponseFlags::EPHEMERAL),
			),
		)
		.await?;

	Ok(())
}

pub async fn print_assignments<'a>(
	ctx: &'a poise::serenity_prelude::Context,
	app: &'a AppState,
	interaction: &'a ComponentInteraction,
) -> Result<(), Error> {
	let mut rows = Vec::new();

	// add one row for each role assignment
	for (id, assignment) in &app.config.assignments {
		let mut options = Vec::new();
		for role in &assignment.roles {
			let mut option = CreateSelectMenuOption::new(role.label.clone(), role.role.to_string()).emoji(role.icon.clone());
			if let Some(subscript) = &role.subscript {
				option = option.description(subscript);
			}

			// preselect roles which user already has
			if let Some(member) = &interaction.member {
				let roles = &member.roles;
				option = option.default_selection(roles.contains(&role.role.into()));
			}
			options.push(option);
		}

		let menu = CreateSelectMenu::new(format!("assign:{}", id), CreateSelectMenuKind::String { options })
			.placeholder(&assignment.title)
			.min_values(0)
			.max_values(assignment.roles.len() as u8);

		rows.push(CreateActionRow::SelectMenu(menu));
	}

	interaction
		.create_response(
			ctx,
			CreateInteractionResponse::Message(
				CreateInteractionResponseMessage::new()
					.content(app.config.self_assignments.prolog.to_string())
					.flags(poise::serenity_prelude::InteractionResponseFlags::EPHEMERAL)
					.components(rows),
			),
		)
		.await?;

	Ok(())
}
