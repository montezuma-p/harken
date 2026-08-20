//! In-process audio decoding to whisper's input format: 16 kHz mono f32.
//!
//! Opus (WhatsApp voice notes) is decoded with libopus straight at 16 kHz —
//! Opus supports native decode rates of 8/12/16/24/48 kHz, so no resampling
//! pass is needed. Everything else goes through symphonia and is resampled
//! with rubato when the source rate differs from 16 kHz.

use std::fs::File;
use std::path::Path;

use rubato::audioadapter_buffers::owned::InterleavedOwned;
use rubato::{Fft, FixedSync, Resampler};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::engine::EngineError;

pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Decode any supported audio file to 16 kHz mono f32 PCM.
pub fn decode_audio_16k_mono(path: &Path) -> Result<Vec<f32>, EngineError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext == "opus" {
        return decode_opus(path);
    }

    match decode_symphonia(path, &ext) {
        Ok(samples) => Ok(samples),
        // .ogg/.oga may carry an opus stream, which symphonia demuxes but
        // cannot decode — fall back to the libopus path.
        Err(e) if ext == "ogg" || ext == "oga" => {
            decode_opus(path).map_err(|opus_err| -> EngineError {
                format!(
                    "failed to decode {}: {e}; opus fallback: {opus_err}",
                    path.display()
                )
                .into()
            })
        }
        Err(e) => Err(e),
    }
}

/// Decode an Ogg Opus file with libopus, directly at 16 kHz.
fn decode_opus(path: &Path) -> Result<Vec<f32>, EngineError> {
    let file = File::open(path)?;
    let mut reader = ogg::PacketReader::new(file);

    // First packet: OpusHead (channel count, pre-skip).
    let head = reader.read_packet()?.ok_or("empty ogg stream")?;
    if head.data.len() < 12 || &head.data[..8] != b"OpusHead" {
        return Err(format!("not an opus stream: {}", path.display()).into());
    }
    let channels = head.data[9] as usize;
    let pre_skip_48k = u16::from_le_bytes([head.data[10], head.data[11]]) as usize;

    let opus_channels = match channels {
        1 => opus::Channels::Mono,
        2 => opus::Channels::Stereo,
        n => return Err(format!("unsupported opus channel count: {n}").into()),
    };
    let mut decoder = opus::Decoder::new(WHISPER_SAMPLE_RATE, opus_channels)?;

    // Second packet: OpusTags (skipped).
    let _tags = reader.read_packet()?;

    // 120 ms is the maximum opus frame duration.
    let max_frames = (WHISPER_SAMPLE_RATE as usize * 120 / 1000) * channels;
    let mut frame_buf = vec![0f32; max_frames];
    let mut samples: Vec<f32> = Vec::new();

    while let Some(packet) = reader.read_packet()? {
        if packet.data.is_empty() {
            continue;
        }
        let decoded_frames = decoder.decode_float(&packet.data, &mut frame_buf, false)?;
        let interleaved = &frame_buf[..decoded_frames * channels];
        if channels == 1 {
            samples.extend_from_slice(interleaved);
        } else {
            samples.extend(
                interleaved
                    .chunks_exact(channels)
                    .map(|frame| frame.iter().sum::<f32>() / channels as f32),
            );
        }
    }

    // Pre-skip is expressed in 48 kHz samples; we decoded at 16 kHz.
    let skip = pre_skip_48k * WHISPER_SAMPLE_RATE as usize / 48_000;
    Ok(samples.split_off(skip.min(samples.len())))
}

/// Decode with symphonia (wav/flac/mp3/m4a/ogg-vorbis/webm/...), downmix to
/// mono, and resample to 16 kHz if needed.
fn decode_symphonia(path: &Path, ext: &str) -> Result<Vec<f32>, EngineError> {
    let src = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    let mut hint = Hint::new();
    if !ext.is_empty() {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| -> EngineError { format!("unsupported format: {e}").into() })?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or("no audio track found")?;
    let track_id = track.id;
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("missing audio codec parameters")?;
    let sample_rate = codec_params.sample_rate.ok_or("unknown sample rate")?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
        .map_err(|e| -> EngineError { format!("unsupported codec: {e}").into() })?;

    let mut mono: Vec<f32> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(format!("demux error: {e}").into()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::IoError(_)) | Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("decode error: {e}").into()),
        };
        let channels = decoded.spec().channels().count();
        decoded.copy_to_vec_interleaved(&mut interleaved);
        if channels <= 1 {
            mono.extend_from_slice(&interleaved);
        } else {
            mono.extend(
                interleaved
                    .chunks_exact(channels)
                    .map(|frame| frame.iter().sum::<f32>() / channels as f32),
            );
        }
    }

    if sample_rate == WHISPER_SAMPLE_RATE {
        return Ok(mono);
    }
    resample_to_16k(mono, sample_rate)
}

fn resample_to_16k(mono: Vec<f32>, from_rate: u32) -> Result<Vec<f32>, EngineError> {
    let input_len = mono.len();
    if input_len == 0 {
        return Ok(mono);
    }
    let mut resampler = Fft::<f32>::new(
        from_rate as usize,
        WHISPER_SAMPLE_RATE as usize,
        1024,
        1,
        FixedSync::Input,
    )
    .map_err(|e| -> EngineError { format!("resampler init failed: {e}").into() })?;

    let input = InterleavedOwned::new_from(mono, 1, input_len)
        .map_err(|e| -> EngineError { format!("resampler buffer error: {e:?}").into() })?;
    let output = resampler
        .process_all(&input, input_len, None)
        .map_err(|e| -> EngineError { format!("resampling failed: {e}").into() })?;
    Ok(output.take_data())
}
