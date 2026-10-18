use simplelog::{ColorChoice, ConfigBuilder, LevelFilter, TermLogger, TerminalMode};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	// OpenDeck writes plugin stdout/stderr to its per-plugin log file.
	let level = if std::env::var_os("KDC_DEBUG").is_some() {
		LevelFilter::Debug
	} else {
		LevelFilter::Info
	};
	let config = ConfigBuilder::new()
		.add_filter_allow_str("kdeconnect_opendeck")
		.add_filter_allow_str("openaction")
		.build();
	if let Err(error) = TermLogger::init(level, config, TerminalMode::Stdout, ColorChoice::Never) {
		eprintln!("logger initialisation failed: {error}");
	}

	kdeconnect_opendeck::actions::register_all().await;
	kdeconnect_opendeck::plugin::start().await?;
	openaction::run(std::env::args().collect()).await?;
	Ok(())
}
