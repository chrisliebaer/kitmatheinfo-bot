#[allow(unused_imports)]
use log::{trace, debug, info, warn, error};
use std::collections::HashSet;
use crate::{AppState, Context, Error};
use poise::{
	serenity_prelude::{
		GuildChannel,
		ButtonStyle,
		CreateActionRow,
		ChannelType,
		CreateSelectMenu,
		CreateSelectMenuOption,
		RoleId,
	},
	FrameworkBuilder,
	Framework,
	serenity::model::interactions::message_component::MessageComponentInteraction,
};
use poise::serenity_prelude::{CreateComponents, Message};

pub fn register_commands(builder: FrameworkBuilder<AppState, Error>) -> FrameworkBuilder<AppState, Error> {
	builder
			.command(post_welcome_message(), |f| f)
			.command(update_welcome_message(), |f| f)
}

fn populate_components(app: &AppState, c: &mut CreateComponents) {
	// adds buttons for toc records
	c.create_action_row(|row| {
		row.create_button(|button| {
			button
					.custom_id("assignments")
					.label(&app.config.self_assignments.label)
					.emoji(app.config.self_assignments.icon.clone())
					.style(ButtonStyle::Success)
		});
		for entry in &app.config.toc {
			row.create_button(|button| {
				button
						.custom_id(format!("toc:{}", entry.file.filename))
						.label(&entry.label)
						.emoji(entry.icon.to_owned())
						.style(ButtonStyle::Primary)
			});
		}
		row
	});
}

/// Aktualisiert die verlinkte Nachricht auf die aktuelle Begrüßung.
#[poise::command(
prefix_command,
rename = "rewelcome",
required_permissions = "MANAGE_GUILD")
]
async fn update_welcome_message(
	ctx: Context<'_>,
	#[description = "Die Nachricht, welche aktualisiert werden soll."]
	mut message: Message,
) -> Result<(), Error> {
	let app = ctx.data();

	let guild = ctx.guild_id().ok_or("not in guild")?;
	let channels = guild.channels(&ctx.discord()).await?;

	if !channels.contains_key(&message.channel_id) {
		return Err(Error::from("target message was not posted in this guild"));
	}

	message.edit(&ctx.discord(), |m| {
		m
				.content(app.config.welcome.to_string())
				.suppress_embeds(true)
				.components(|c| {
					populate_components(app, c);
					c
				})
	}).await?;

	ctx.send(|m| {
		m.content("Nachricht erfolgreich aktualisiert.").ephemeral(true)
	}).await?;

	Result::Ok(())
}

/// Erstellt die Begrüßungsnachricht im angegebenen Channel.
#[poise::command(
prefix_command,
rename = "welcome",
required_permissions = "MANAGE_GUILD")
]
async fn post_welcome_message(
	ctx: Context<'_>,
	#[description = "Der Channel in dem Nachricht erstellt werden soll."]
	channel: GuildChannel,
) -> Result<(), Error> {
	let app = ctx.data();

	if ctx.guild_id().ok_or("not in guild")? != channel.guild_id {
		return Err(Error::from("current guild differs from guild of target channel"));
	}

	// TODO: this can be done by using the "channel_types" field, but is not supported by poise
	if channel.kind != ChannelType::Text {
		return Err(Error::from("not a text channel"));
	}

	channel.send_message(&ctx.discord(), |m| {
		m
				.content(app.config.welcome.to_string())
				.components(|c| {
					populate_components(app, c);
					c
				})
	}).await?;

	ctx.send(|m| {
		m.content("Nachricht erfolgreich erstellt").ephemeral(true)
	}).await?;

	Result::Ok(())
}

pub async fn handle_assign_click<'a>(
	ctx: &'a poise::serenity_prelude::Context,
	_framework: &'a Framework<AppState, Error>,
	app: &'a AppState,
	interaction: &'a MessageComponentInteraction,
) -> Result<(), Error> {
	let data = &interaction.data;

	// toc buttons are identified as `toc:$file`
	let custom_id = data.custom_id.as_str();
	let id = custom_id.splitn(2, ":").last()
			.ok_or(format!("Unknown format in toc custom_id: {}", custom_id))?;

	let assignment = app.config.assignments.get(id).ok_or(format!("Unknown assignment: {}", id))?;
	let member = interaction.member.as_ref().ok_or("not executed in guild, no way to assign roles")?;

	// calculate ids of all roles in assignment
	let all_roles = &assignment.roles.iter().map(|a| a.role).collect::<HashSet<_>>();

	// menu options contain corresponding roles, so extract them
	let selected = data.values.iter().map(|x| x.parse::<u64>()).collect::<Result<HashSet<u64>, _>>()?;

	// current roles of user, important since discord will reject modifications with preexisting role assignments
	let current = member.roles.iter().map(|x| x.as_u64().to_owned()).collect::<HashSet<u64>>();

	// remove all roles which are not selected, but only if user currently has them
	let removed_roles = &(all_roles - &selected) & &current;

	// add all selected roles but only if user does not already have them
	let new_roles = &selected - &current;

	// convert these sets to vec since we need slices for api calls
	// TODO: this is probably a much nicer way to accomplish the same thing
	let new_roles = new_roles.into_iter().map(|x| x.to_owned().into()).collect::<Vec<RoleId>>();
	let removed_roles = removed_roles.into_iter().map(|x| x.to_owned().into()).collect::<Vec<RoleId>>();

	// we need to react to the interaction since role update could cause too much delay
	interaction.create_interaction_response(ctx, |resp| {
		resp.interaction_response_data(|d| {
			d
					.content(format!("Ich aktualisiere deine Rollen..."))
					.flags(poise::serenity_prelude::InteractionApplicationCommandCallbackDataFlags::EPHEMERAL)
		})
	}).await?;

	// apply role modifications (remove comes first to prevent permission excalation between updates)
	let mut member = member.clone();
	member.remove_roles(ctx, removed_roles.as_slice()).await.unwrap();
	member.add_roles(ctx, new_roles.as_slice()).await.unwrap();

	// update initial response and notify user about success
	interaction.edit_original_interaction_response(ctx, |resp| {
		let new_roles = new_roles.into_iter().map(|role| format!("<@&{}>", role.to_string())).collect::<Vec<_>>().join(", ");
		let removed_roles = removed_roles.into_iter().map(|role| format!("<@&{}>", role.to_string())).collect::<Vec<_>>().join(", ");

		resp
				// disable mention since we are abount to mention A LOT of roles
				.allowed_mentions(|mentions| { mentions.empty_parse() })
				.content(format!(r#"
**Rollen erfolgreich angepasst**
Neue Rollen: {}

Entfernte Rollen: {}
				"#, new_roles, removed_roles))
	}).await.unwrap();

	Ok(())
}

pub async fn handle_toc_click<'a>(
	ctx: &'a poise::serenity_prelude::Context,
	_framework: &'a Framework<AppState, Error>,
	app: &'a AppState,
	interaction: &'a MessageComponentInteraction,
) -> Result<(), Error> {
	let data = &interaction.data;

	// assign buttons are identified as `assign:$file`
	let custom_id = data.custom_id.as_str();
	let file = custom_id.splitn(2, ":").last()
			.ok_or(format!("Unknown format in assign custom_id: {}", custom_id))?;

	let entry = app.config.toc.iter()
			.find(|f| f.file.filename == file).ok_or(format!("Unknown toc file: {}", file))?;

	interaction.create_interaction_response(ctx, |f| {
		f.interaction_response_data(|d| {
			d
					.content(entry.file.content.as_str())
					.flags(poise::serenity_prelude::InteractionApplicationCommandCallbackDataFlags::EPHEMERAL)
		})
	}).await?;

	Ok(())
}

pub async fn print_assignments<'a>(
	ctx: &'a poise::serenity_prelude::Context,
	_framework: &'a Framework<AppState, Error>,
	app: &'a AppState,
	interaction: &'a MessageComponentInteraction,
) -> Result<(), Error> {
	interaction.create_interaction_response(ctx, |f| {
		f.interaction_response_data(|d| {
			d
					.content(&app.config.self_assignments.prolog)
					.flags(poise::serenity_prelude::InteractionApplicationCommandCallbackDataFlags::EPHEMERAL)
					.components(|c| {

						// add one row for each role assignment
						for (id, assignment) in &app.config.assignments {
							let mut menu = CreateSelectMenu::default();
							menu.custom_id(format!("assign:{}", id));
							menu.placeholder(&assignment.title);
							menu.min_values(0);
							menu.max_values(assignment.roles.len() as u64);
							menu.options(|opts| {
								for role in &assignment.roles {
									let mut option = CreateSelectMenuOption::default();
									option.label(&role.label);
									option.emoji(role.icon.clone());
									option.value(&role.role);
									if let Some(subscript) = &role.subscript {
										option.description(subscript);
									}

									// preselect roles which user already has
									if let Some(member) = &interaction.member {
										let roles = &member.roles;
										option.default_selection(roles.contains(&role.role.into()));
									}

									opts.add_option(option);
								}
								opts
							});

							let mut row = CreateActionRow::default();
							row.add_select_menu(menu);
							c.add_action_row(row);
						};
						c
					})
		})
	}).await?;

	Ok(())
}
