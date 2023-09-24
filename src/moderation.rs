use poise::Command;
use serenity::model::prelude::ChannelId;
use serenity::prelude::Mentionable;
use crate::{AppState, Context, Error};

const REPORT_MESSAGE_LENGTH: usize = 500;

pub fn register_commands(commands: &mut Vec<Command<AppState, Error>>) {
	commands.push(report_message());
}


#[derive(Debug, poise::Modal)]
#[name = "Nachricht melden"]
struct ModalReport {
	#[name = "Grund"]
	#[placeholder = "Nenne zusätzliche Informationen und den Grund für die Meldung."]
	#[paragraph]
	#[min_length = 5]
	#[max_length = 500]
	reason: String,
}

/// Erstellt einen neuen Kanal.
#[poise::command(context_menu_command = "Nachricht melden", ephemeral)]
async fn report_message(
	ctx: Context<'_>,
	#[description = "Nachricht"] msg: poise::serenity_prelude::Message,
) -> Result<(), Error> {
	let app_context = match ctx {
		Context::Application(ctx) => ctx,
		Context::Prefix(_) => unreachable!("This command is only available as a context menu command")
	};
	let report_channel = match app_context.data().config.moderation.report_channel {
		Some(id) => ChannelId(id),
		None => {
			ctx.send(|m| {
				m.content("Die Meldefunktion ist nicht aktiviert.")
			}).await?;
			return Ok(());
		}
	};

	let report = poise::execute_modal::<_, _, ModalReport>(app_context, None, None).await?;

	match report {
		Some(_) => {
			report_channel.send_message(ctx, |m| {
				m.embed(|e| {
					let message_abbreviation = if msg.content.len() > REPORT_MESSAGE_LENGTH {
						&msg.content[..REPORT_MESSAGE_LENGTH]
					} else {
						&msg.content
					};

					e.title(format!("Neue Meldung von {}", ctx.author().name))
							.description(&message_abbreviation)
							.field("Grund", report.unwrap().reason, true)
							.field("Link", format!("[Link]({})", msg.link()), true)
							.field("Autor", msg.author.mention(), true)
							.field("Kanal", msg.channel_id.mention(), true)
							.field("Melder", ctx.author().mention(), true)
							.timestamp(msg.timestamp.to_rfc3339())
				})
			}).await?;

			ctx.send(|m| {
				m.content("Die Nachricht wurde gemeldet.")
			}).await?;
		}
		None => {
			ctx.send(|m| {
				m.content("Du hast die Meldung abgebrochen oder es trat ein Fehler auf.")
			}).await?;
			return Ok(());
		}
	}

	Ok(())
}
