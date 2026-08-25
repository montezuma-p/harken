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
            // Warm the model cache on a side thread so the read loop answers
            // initialize/discover/tools/list in milliseconds while the
            // download streams; the WarmGate holds tool calls until it
            // settles. A failed warm-up is logged and the gate lets calls
            // through to retry inline — it must never kill the server.
            let warmth = harken::mcp::Warmth::new();
            let warm_handle = {
                let warmth = warmth.clone();
                let model = args.model.clone();
                std::thread::spawn(move || {
                    // The guard settles the state on unwind, so a panic in here
                    // cannot leave the gate blocking every tool call forever.
                    let settle = harken::mcp::WarmSettle::new(warmth.clone());
                    let mut sink = harken::model::StderrSink::default();
                    match harken::model::ensure_downloaded(&model, &mut sink) {
                        Ok(_) => warmth.set_ready(),
                        Err(e) => {
                            eprintln!("warning: model warm-up failed: {e}");
                            warmth.set_failed(e);
                        }
                    }
                    settle.done();
                })
            };
            let info = harken::mcp::ServerInfo {
                model: args.model.clone(),
                lang: args.lang.clone(),
                device: args.device.clone(),
            };
            let mut engine = WhisperCppEngine::new(
                args.model.clone(),
                args.device.clone(),
                language_option(&args.lang),
            );
            let mut gate = harken::mcp::WarmGate::new(&mut engine, warmth);
            let mut stdout = std::io::stdout().lock();
            let code = match harken::mcp::serve_with_info(
                std::io::stdin().lock(),
                &mut stdout,
                &mut gate,
                &info,
            ) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            };
            // Detach rather than join: EOF means the client is gone, and
            // joining would hold the process open for the rest of a download
            // nobody is waiting on — up to 466 MB for the default `small`, once
            // per server a client spawns and kills while probing a config.
            //
            // Abandoning a download can never corrupt the cache: only a
            // Content-Length- and SHA-256-checked file is ever renamed into
            // place, so a killed download leaves nothing a later run can mistake
            // for a model. The honest cost is debris — process::exit runs no
            // destructor, so PartialGuard does not fire and the
            // `.partial-<pid>-<n>` file is orphaned in the cache dir. The nonce
            // keeps it inert; it is wasted bytes, not a wrong model. Bounding
            // shutdown is worth that.
            drop(warm_handle);
            code
        }
        Some(Commands::Warm(args)) => {
            let mut sink = harken::model::BarSink::default();
            match harken::model::ensure_downloaded(&args.model, &mut sink) {
                Ok(path) => {
                    eprintln!("model {} ready at {}", args.model, path.display());
                    0
                }
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
