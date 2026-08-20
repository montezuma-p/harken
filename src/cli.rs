//! Command-line surface: batch mode and the whatsapp subcommand.

use clap::{Args, Parser, Subcommand};

use crate::writers::OutputFormat;

#[derive(Parser, Debug)]
#[command(
    name = "harken",
    version,
    about = "Local audio transcription CLI (whisper.cpp, offline)",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[command(flatten)]
    pub batch: BatchArgs,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Transcribe audio attachments from a WhatsApp chat export
    Whatsapp(WhatsappArgs),
}

#[derive(Args, Debug)]
pub struct BatchArgs {
    /// Audio files, directories, or globs to transcribe
    #[arg(required = true)]
    pub inputs: Vec<String>,

    /// Output directory
    #[arg(long, default_value = "./transcripts")]
    pub out: String,

    /// Whisper model size (or a path to a ggml .bin file)
    #[arg(long, default_value = "small")]
    pub model: String,

    /// Language code, or 'auto' to auto-detect
    #[arg(long, default_value = "pt")]
    pub lang: String,

    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Txt)]
    pub format: OutputFormat,

    /// Device to run on
    #[arg(long, default_value = "cpu")]
    pub device: String,

    /// Re-transcribe even if output already exists
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct WhatsappArgs {
    /// Path to the WhatsApp chat export .zip
    pub export_zip: String,

    /// Output directory (default: ./<zip-stem>-transcripts)
    #[arg(long)]
    pub out: Option<String>,

    /// Only include messages on/after this date (YYYY-MM-DD)
    #[arg(long = "from", value_name = "YYYY-MM-DD")]
    pub date_from: Option<String>,

    /// Only include messages on/before this date (YYYY-MM-DD)
    #[arg(long = "to", value_name = "YYYY-MM-DD")]
    pub date_to: Option<String>,

    /// Write out_dir/_chat.transcribed.txt with transcripts inlined
    #[arg(long)]
    pub merge: bool,

    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Txt)]
    pub format: OutputFormat,

    /// Whisper model size (or a path to a ggml .bin file)
    #[arg(long, default_value = "small")]
    pub model: String,

    /// Language code, or 'auto' to auto-detect
    #[arg(long, default_value = "pt")]
    pub lang: String,

    /// Device to run on
    #[arg(long, default_value = "cpu")]
    pub device: String,

    /// Re-transcribe even if output already exists
    #[arg(long)]
    pub force: bool,
}

/// `--lang auto` means "let the engine detect the language".
pub fn language_option(lang: &str) -> Option<String> {
    if lang == "auto" {
        None
    } else {
        Some(lang.to_string())
    }
}
