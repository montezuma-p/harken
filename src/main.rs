use clap::Parser;

use harken::batch::run_batch_mode;
use harken::cli::{Cli, Commands, language_option};
use harken::engine::WhisperCppEngine;

fn main() {
    let cli = Cli::parse();

    let code = match &cli.command {
        Some(Commands::Whatsapp(args)) => {
            let mut engine = WhisperCppEngine::new(
                args.model.clone(),
                args.device.clone(),
                language_option(&args.lang),
            );
            harken::whatsapp::run(args, &mut engine)
        }
        Some(Commands::Mcp(args)) => {
            let mut engine = WhisperCppEngine::new(
                args.model.clone(),
                args.device.clone(),
                language_option(&args.lang),
            );
            let mut stdout = std::io::stdout().lock();
            match harken::mcp::serve(std::io::stdin().lock(), &mut stdout, &mut engine) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        None => {
            let args = &cli.batch;
            let mut engine = WhisperCppEngine::new(
                args.model.clone(),
                args.device.clone(),
                language_option(&args.lang),
            );
            run_batch_mode(
                &args.inputs,
                &args.out,
                args.format,
                args.force,
                &mut engine,
            )
        }
    };

    std::process::exit(code);
}
