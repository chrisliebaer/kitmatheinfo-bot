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
use poise::{ErrorContext, Event, Framework, serenity_prelude::GatewayIntents};

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
		"Mit 'help <Befehl>' bekommst du weitere Hilfe zu Befehlen. Außerdem kannst du Befehle auch über einen Slash (/) verwenden.",
		poise::builtins::HelpResponseMode::Ephemeral,
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
	framework: &'a Framework<AppState, Error>,
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
						toc::handle_toc_click(ctx, framework, app, component_interaction).await?;
					} else if custom_id.starts_with("assignments") {
						toc::print_assignments(ctx, framework, app, component_interaction).await?;
					} else if custom_id.starts_with("assign:") {
						toc::handle_assign_click(ctx, framework, app, component_interaction).await?;
					}
				}
				_ => ()
			};
		}
		Ready { data_about_bot } => info!("Bot is ready: {:?}", data_about_bot),
		_ => ()
	};
	Ok(())
}

async fn on_error(error: Error, ctx: poise::ErrorContext<'_, AppState, Error>) {
	use ErrorContext::*;
	match ctx {
		Setup =>
			panic!("Failed to start bot: {:?}", error),
		Command(ctx) => {
			let send_result = ctx.ctx().send(|m| {
				m.embed(|e| {
					e.title("Fehler").description(&error)
				}).ephemeral(true)
			}).await;
			if let Err(_) = send_result {
				error!("Error while handling error: {:?}", error);
			};
			error!("Error in command `{}`: {:?}", ctx.command().name(), error);
		}

		Listener(event) => error!("Error handling event: {}: for event {:?}", error, event),
		Autocomplete(ctx) =>
			error!("Error in auto-completion for command `{}`: {:?}", ctx.ctx.command.slash_or_context_menu_name(), error),
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

	let builder = poise::Framework::build()
			.token(&config.bot_token)
			.client_settings(|b| {
				b.intents(
					GatewayIntents::GUILDS |
							GatewayIntents::GUILD_MESSAGES |
							GatewayIntents::DIRECT_MESSAGES |
							GatewayIntents::GUILD_INTEGRATIONS
				)
			})
			.user_data_setup(move |_ctx, _ready, _framework| Box::pin(async move {
				Ok(AppState {
					config,
				})
			}))
			.command(help(), |f| f)
			.command(register(), |f| f)
			.options(poise::FrameworkOptions {
				listener: |ctx, ev, framework, app| { Box::pin(listener(ctx, ev, framework, app)) },
				prefix_options: poise::PrefixFrameworkOptions {
					mention_as_prefix: true,
					..Default::default()
				},
				on_error: |error, ctx| Box::pin(on_error(error, ctx)),
				pre_command: |ctx| {
					Box::pin(async move {
						trace!("Executing command {}...", ctx.command().unwrap().name());
					})
				},
				post_command: |ctx| {
					Box::pin(async move {
						trace!("Executed command {}!", ctx.command().unwrap().name());
					})
				},
				..Default::default()
			});

	// TODO: perform proper shutdown
	let builder = toc::register_commands(builder);
	let builder = self_management::register_commands(builder);
	builder.run().await.unwrap();
}
