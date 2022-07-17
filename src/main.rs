mod config;
mod toc;
mod self_management;

use env_logger::Target;
#[allow(unused_imports)]
use log::{trace, debug, info, warn, error};
use config::Config;
use std::{
	fs::File,
	io::Read,
};
use poise::{FrameworkError, Event, Framework, serenity_prelude::GatewayIntents, FrameworkOptions};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, AppState, Error>;

pub struct AppState {
	config: Config,
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
			extra_text_at_bottom: "Mit 'help <Befehl>' bekommst du weitere Hilfe zu Befehlen. Außerdem kannst du Befehle auch über einen Slash (/) verwenden.",
			show_context_menu_commands: true,
			..Default::default()
		},
	).await?;
	Ok(())
}

/// Aktualisiert die registrierten Befehle des Bots. Kann nur vom Besitzer ausgeführt werden.
#[poise::command(prefix_command, hide_in_help)]
async fn register(ctx: Context<'_>, #[flag] global: bool) -> Result<(), Error> {
	poise::builtins::register_application_commands(ctx, global).await?;
	Ok(())
}

/// Generic listener on top of poise to handle all incoming discord events. Especially button interactions, which pose doesn't support yet.
async fn listener<'a>(
	ctx: &'a poise::serenity_prelude::Context,
	ev: &'a Event<'a>,
	app: &'a AppState,
) -> Result<(), Error> {
	use poise::{
		Event::InteractionCreate,
		Event::Ready,
		serenity_prelude::Interaction::MessageComponent,
	};
	match ev {
		InteractionCreate { interaction } => {
			match interaction {
				MessageComponent(component_interaction) => {
					let custom_id = component_interaction.data.custom_id.as_str();
					if custom_id.starts_with("toc:") {
						toc::handle_toc_click(ctx, app, component_interaction).await?;
					} else if custom_id.starts_with("assignments") {
						toc::print_assignments(ctx, app, component_interaction).await?;
					} else if custom_id.starts_with("assign:") {
						toc::handle_assign_click(ctx, app, component_interaction).await?;
					}
				}
				_ => (),
			};
			trace!("Incoming interaction: {:?}", interaction)
		}
		Ready { data_about_bot } => info!("Bot is ready: {:?}", data_about_bot),
		_ => (),
	};
	Ok(())
}

async fn on_error(error: FrameworkError<'_, AppState, Error>) {
	use FrameworkError::*;
	match error {
		Setup { error } => panic!("Failed to start bot: {:?}", error),
		Command { error, ctx } => {
			let send_result = ctx.send(|m| {
				m.embed(|e| {
					e.title("Fehler").description(&error)
				}).ephemeral(true)
			}).await;
			if let Err(_) = send_result {
				error!("Error while handling error: {:?}", error);
			};
			error!("Error in command `{}`: {:?}", ctx.command().name, error);
		}
		error => {
			if let Err(e) = poise::builtins::on_error(error).await {
				println!("Error while handling error: {}", e)
			}
		}
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

	let mut commands: Vec<_> = vec![
		help(),
		register(),
	];

	toc::register_commands(&mut commands);
	self_management::register_commands(&mut commands);

	let options = FrameworkOptions {
		commands,
		listener: |ctx, ev, _framework, app| {
			Box::pin(listener(ctx, ev, app))
		},
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

	Framework::build()
			.token(&config.bot_token)
			.intents(GatewayIntents::GUILDS |
					GatewayIntents::GUILD_MESSAGES |
					GatewayIntents::DIRECT_MESSAGES |
					GatewayIntents::GUILD_INTEGRATIONS)
			.user_data_setup(move |_ctx, _ready, _framework| Box::pin(async move {
				Ok(AppState {
					config,
				})
			}))
			.options(options)
			.run().await.unwrap();
}
