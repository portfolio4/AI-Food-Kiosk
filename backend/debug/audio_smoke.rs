use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};
use audiopus::{Application, Channels, SampleRate, coder::Encoder};
use simple_stt::STTDecoder;

const OPUS_FRAME_SAMPLES: usize = 960;

fn main() -> Result<()> {
    let wav_path = env::args()
        .nth(1)
        .context("usage: cargo run --example audio_smoke -- <48-kHz-mono.wav>")?;
    let mut reader = hound::WavReader::open(&wav_path).context("failed to open WAV")?;
    let spec = reader.spec();
    if spec.sample_rate != 48_000 || spec.channels != 1 || spec.bits_per_sample != 16 {
        bail!("fixture must be 48 kHz, mono, 16-bit PCM");
    }

    let samples = reader
        .samples::<i16>()
        .map(|sample| sample.map(|value| value as f32 / i16::MAX as f32))
        .collect::<Result<Vec<_>, _>>()?;
    let encoder = Encoder::new(SampleRate::Hz48000, Channels::Mono, Application::Voip)?;
    let mut encoded = vec![0_u8; 4_000];
    let models_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models");
    let mut decoder = STTDecoder::builder()
        .language("en")
        .whisper_model_path(models_dir.join("ggml-tiny.en.bin"))
        .vad_model_path(models_dir.join("ggml-silero-v6.2.0.bin"))
        .build()?;

    for input in samples.chunks(OPUS_FRAME_SAMPLES) {
        let mut frame = [0_f32; OPUS_FRAME_SAMPLES];
        frame[..input.len()].copy_from_slice(input);
        let encoded_len = encoder.encode_float(&frame, &mut encoded)?;
        let output = decoder.stream(&encoded[..encoded_len])?;

        if let Some(text) = output.committed_text {
            println!("committed_text: {text}");
        }
        if let Some(text) = output.tentative_text {
            println!("tentative_text: {text}");
        }
    }

    Ok(())
}
