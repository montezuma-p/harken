//! Offline fixture-based tests for src/audio.rs — no real model, no network.
//!
//! These pin the "nothing decoded is an error" contract from issue #11: a
//! file symphonia can probe but not decode must fail, not transcribe as
//! empty with exit 0.

use std::path::PathBuf;

use harken::audio::decode_audio_16k_mono;

/// Valid MPEG-1 Layer III frame headers with filler payload: symphonia's
/// probe accepts it, but every packet fails to decode.
fn synced_garbage_mp3(dir: &std::path::Path) -> PathBuf {
    let mut frame = vec![0xFFu8, 0xFB, 0x90, 0x00];
    frame.extend(std::iter::repeat_n(0x55u8, 413));
    let path = dir.join("synced_garbage.mp3");
    std::fs::write(&path, frame.repeat(200)).unwrap();
    path
}

/// Minimal valid mono 16 kHz PCM16 WAV with `n` samples.
fn wav_16k_mono(dir: &std::path::Path, n: usize) -> PathBuf {
    let data_len = (n * 2) as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&16_000u32.to_le_bytes()); // sample rate
    bytes.extend_from_slice(&32_000u32.to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..n {
        let v = ((i % 100) as i16) * 300;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let path = dir.join(format!("tone_{n}.wav"));
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn probe_ok_but_every_packet_undecodable_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let path = synced_garbage_mp3(tmp.path());

    let err = decode_audio_16k_mono(&path).unwrap_err();

    assert!(
        err.to_string().contains("no audio decoded"),
        "\"silence\" and \"nothing decoded\" must not be the same outcome: {err}"
    );
}

#[test]
fn file_decoding_to_zero_samples_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let path = wav_16k_mono(tmp.path(), 0);

    let err = decode_audio_16k_mono(&path).unwrap_err();

    assert!(
        err.to_string().contains("zero samples"),
        "an empty decode must not flow through as an empty transcript: {err}"
    );
}

#[test]
fn valid_wav_decodes_all_samples() {
    let tmp = tempfile::tempdir().unwrap();
    let path = wav_16k_mono(tmp.path(), 1600); // 0.1 s

    let samples = decode_audio_16k_mono(&path).unwrap();

    assert_eq!(samples.len(), 1600);
    assert!(samples.iter().any(|s| *s != 0.0));
}
