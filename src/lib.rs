//! harken — local, offline audio transcription (whisper.cpp) with
//! WhatsApp chat-export support.

pub mod audio;
pub mod batch;
pub mod cli;
pub mod engine;
mod ffi;
pub mod model;
pub mod whatsapp;
pub mod writers;
