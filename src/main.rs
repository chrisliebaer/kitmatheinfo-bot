mod config;
mod moderation;
mod ophase;
mod self_management;
mod toc;

use std::{
	fs::File,
	io::Read,
	sync::{Arc, Mutex},
};

use config::Config;
use env_logger::Target;
#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};
use poise::{serenity_prelude::GatewayIntents, CreateReply, Framework, FrameworkError, FrameworkOptions};
use serenity::all::{ClientBuilder, CreateEmbed, FullEvent, TeamMemberRole, User};

use crate::ophase::get_ophase_invite_count;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, AppState, Error>;

pub struct AppState {
	config: Config,
	ophase_invite_uses: Arc<Mutex<Option<u64>>>,
}

/// Show this help menu
#[poise::command(prefix_command, slash_command, track_edits)]
async fn help(
	ctx: Context<'_>,
	#[description = "Befehl zu dem du Hilfe benötigst."]
	#[autocomplete = "poise::builtins::autocomplete_command"]
	command: Option<String>,
) -> Result<(), Error> {
	poise::builtins::help(
		ctx,
		command.as_deref(),
		poise::builtins::HelpConfiguration {
			extra_text_at_bottom: "Mit 'help <Befehl>' bekommst du weitere Hilfe zu Befehlen. Außerdem kannst du Befehle auch über \
		                       einen Slash (/) verwenden.",
			show_context_menu_commands: true,
			..Default::default()
		},
	)
	.await?;
	Ok(())
}

async fn is_bot_team_admin_or_owner(ctx: Context<'_>, potential_owner: &User) -> Result<bool, Error> {
	let app_info = ctx.http().get_current_application_info().await?;

	// Normal owner
	if let Some(user) = app_info.owner
		&& user.id == potential_owner.id
	{
		return Ok(true);
	}

	if let Some(team) = app_info.team {
		// Team owner
		if team.owner_user_id == potential_owner.id {
			return Ok(true);
		}

		// Admin in team
		for member in team.members {
			if member.user.id != potential_owner.id {
				continue;
			}
			if matches!(member.role, TeamMemberRole::Admin) {
				return Ok(true);
			}
		}
	}

	Ok(false)
}

/// Aktualisiert die registrierten Befehle des Bots. Kann nur vom Besitzer ausgeführt werden.
#[poise::command(prefix_command, hide_in_help)]
async fn register(ctx: Context<'_>, #[flag] global: bool) -> Result<(), Error> {
	let is_bot_owner = is_bot_team_admin_or_owner(ctx, ctx.author()).await?;
	if !is_bot_owner {
		ctx.say("Can only be used by bot owner").await?;
		return Ok(());
	}

	let commands_builder = poise::builtins::create_application_commands(&ctx.framework().options().commands);
	let num_commands = commands_builder.len();

	if global {
		ctx.say(format!("Registering {num_commands} global commands...",)).await?;
		poise::serenity_prelude::Command::set_global_commands(ctx, commands_builder).await?;
	} else {
		let guild_id = match ctx.guild_id() {
			Some(x) => x,
			None => {
				ctx.say("Must be called in guild").await?;
				return Ok(());
			},
		};

		ctx.say(format!("Registering {num_commands} guild commands...")).await?;
		guild_id.set_commands(ctx, commands_builder).await?;
	}

	ctx.say("Done!").await?;

	Ok(())
}

/// Generic listener on top of poise to handle all incoming discord events. Especially button interactions, which pose
/// doesn't support yet.
async fn listener<'a>(ctx: &'a poise::serenity_prelude::Context, ev: &'a FullEvent, app: &'a AppState) -> Result<(), Error> {
	use serenity::model::application::Interaction;
	match ev {
		FullEvent::InteractionCreate { interaction } => {
			if let Interaction::Component(component_interaction) = &interaction {
				let custom_id = component_interaction.data.custom_id.as_str();
				if custom_id.starts_with("toc:") {
					toc::handle_toc_click(ctx, app, component_interaction).await?;
				} else if custom_id.starts_with("assignments") {
					toc::print_assignments(ctx, app, component_interaction).await?;
				} else if custom_id.starts_with("assign:") {
					toc::handle_assign_click(ctx, app, component_interaction).await?;
				}
			};
			trace!("Incoming interaction: {:?}", interaction)
		},
		FullEvent::GuildMemberAddition { new_member } => {
			if let Some(o_phase_config) = &app.config.o_phase {
				ophase::handle_new_guild_member(ctx, new_member, o_phase_config, &app.ophase_invite_uses).await?;
			}
		},
		FullEvent::Ready { data_about_bot } => info!("Bot is ready: {:?}", data_about_bot),
		_ => (),
	};
	Ok(())
}

async fn on_error(error: FrameworkError<'_, AppState, Error>) {
	use FrameworkError::*;
	match error {
		Setup { error, .. } => panic!("Failed to start bot: {:?}", error),
		Command { error, ctx, .. } => {
			let send_result = ctx
				.send(
					CreateReply::default()
						.embed(CreateEmbed::new().title("Fehler").description(error.to_string()))
						.ephemeral(true),
				)
				.await;
			if send_result.is_err() {
				error!("Error while handling error: {:?}", error);
			};
			error!("Error in command `{}`: {:?}", ctx.command().name, error);
		},
		error => {
			if let Err(e) = poise::builtins::on_error(error).await {
				println!("Error while handling error: {}", e)
			}
		},
	}
}

#[tokio::main]
async fn main() {
	env_logger::builder()
		.parse_default_env()
		.format_timestamp(None)
		.target(Target::Stdout)
		.init();

	let args = std::env::args().collect::<Vec<_>>();
	let file = args.get(1).expect("No config file given");
	let mut file = File::open(file).unwrap();
	let mut content = String::new();
	file.read_to_string(&mut content).unwrap();
	let config = toml::from_str::<Config>(content.as_str()).unwrap();

	info!("This is a log message and we need it!");

	let mut commands: Vec<_> = vec![help(), register()];

	toc::register_commands(&mut commands);
	self_management::register_commands(&mut commands);
	moderation::register_commands(&mut commands);
	ophase::register_commands(&mut commands);

	let options = FrameworkOptions {
		commands,
		event_handler: |ctx, ev, _framework, app| Box::pin(listener(ctx, ev, app)),
		prefix_options: poise::PrefixFrameworkOptions {
			mention_as_prefix: true,
			..Default::default()
		},
		pre_command: |ctx| {
			Box::pin(async move {
				trace!("Executing command {}... for {}", ctx.command().name, ctx.author());
			})
		},
		post_command: |ctx| {
			Box::pin(async move {
				trace!("Executed command {}!", ctx.command().name);
			})
		},
		on_error: |error| Box::pin(on_error(error)),
		..Default::default()
	};

	let bot_token = config.bot_token.clone();

	let framework = Framework::builder()
		.setup(move |ctx, ready, _framework| {
			Box::pin(async move {
				let ophase_invite_count = get_ophase_invite_count(ctx, ready, &config).await;
				let ophase_invite_count = Arc::new(Mutex::new(ophase_invite_count));
				Ok(AppState {
					config,
					ophase_invite_uses: ophase_invite_count,
				})
			})
		})
		.options(options)
		.build();

	let client = ClientBuilder::new(
		bot_token,
		GatewayIntents::GUILDS
			| GatewayIntents::GUILD_MESSAGES
			| GatewayIntents::DIRECT_MESSAGES
			| GatewayIntents::GUILD_INTEGRATIONS
			| GatewayIntents::GUILD_MEMBERS,
	)
	.framework(framework)
	.await;

	client.unwrap().start().await.unwrap();
}
