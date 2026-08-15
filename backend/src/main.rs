use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use realtime::{RealtimeServer, RealtimeSession};
use serde_json::json;
use simple_rtc::{AudioTrack, LiveAudioRTCServer};
use simple_stt::STTDecoder;
use tokio::{
    sync::Mutex,
    time::{Instant, timeout_at},
};

const SIGNALING_ADDRESS: &str = "127.0.0.1:9001";
const REALTIME_ADDRESS: &str = "127.0.0.1:9002";
const AGENT_INITIAL_PROMPT: &str = include_str!("../ai_kiosk_prompt.txt");
const RTP_BUFFER_SIZE: usize = 1500;
const TEXT_IDLE_TIMEOUT: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> Result<()> {
    let models_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models");
    let decoder = Arc::new(Mutex::new(
        STTDecoder::builder()
            .language("en")
            .whisper_model_path(models_dir.join("ggml-tiny.en.bin"))
            .vad_model_path(models_dir.join("ggml-silero-v6.2.0.bin"))
            .build()?,
    ));
    let realtime_server = RealtimeServer::new(
        REALTIME_ADDRESS,
        AGENT_INITIAL_PROMPT,
        |session: RealtimeSession, message: serde_json::Value| async move {
            match message.get("msg_type").and_then(serde_json::Value::as_str) {
                Some("next_customer") => {
                    session.agent().prompt("[NEXT_CUSTOMER]".to_owned(), |_| {});
                }
                Some(msg_type) => {
                    eprintln!(
                        "Ignoring unknown realtime message from {}: {msg_type}",
                        session.client_id(),
                    );
                }
                None => {
                    eprintln!(
                        "Ignoring realtime message without msg_type from {}",
                        session.client_id(),
                    );
                }
            }
        },
    );
    let rtc_decoder = Arc::clone(&decoder);
    let rtc_server = LiveAudioRTCServer::new(
        SIGNALING_ADDRESS,
        realtime_server.sessions(),
        move |track: AudioTrack, session: RealtimeSession| {
            let decoder = Arc::clone(&rtc_decoder);
            async move {
                if let Err(error) = transcribe_audio_track(track, session, decoder).await {
                    eprintln!("Audio transcription failed: {error:#}");
                }
            }
        },
    );

    tokio::try_join!(realtime_server.run(), rtc_server.run())?;
    Ok(())
}

async fn transcribe_audio_track(
    track: AudioTrack,
    session: RealtimeSession,
    decoder: Arc<Mutex<STTDecoder>>,
) -> Result<()> {
    let mut rtp_buffer = vec![0_u8; RTP_BUFFER_SIZE];
    let mut committed_text = Vec::new();

    loop {
        let mut decoder = decoder.lock().await;
        let mut text_deadline = Instant::now() + TEXT_IDLE_TIMEOUT;

        loop {
            match timeout_at(text_deadline, track.read(&mut rtp_buffer)).await {
                Ok(Ok((rtp_packet, _attributes))) => {
                    let output = decoder.stream(rtp_packet.payload.as_ref())?;

                    if output.has_text() {
                        text_deadline = Instant::now() + TEXT_IDLE_TIMEOUT;
                    }
                    if let Some(text) = output.committed_text {
                        println!("buffered_commited_text: {text}");
                        session.send(&json!({
                            "msg_type": "buffered_committed_text",
                            "text": text,
                        }))?;
                        committed_text.push(text);
                    }
                }
                Ok(Err(error)) => {
                    commit_text(&mut committed_text, &session)?;
                    return Err(error).context("failed to read RTC audio track");
                }
                Err(_) => {
                    commit_text(&mut committed_text, &session)?;
                    break;
                }
            }
        }
    }
}

fn commit_text(committed_text: &mut Vec<String>, session: &RealtimeSession) -> Result<()> {
    if committed_text.is_empty() {
        return Ok(());
    }

    let prompt = committed_text.join(" ");
    println!("{prompt}");
    let response_session = session.clone();
    session.agent().prompt(prompt, move |response| {
        if let Err(error) = response_session.send(&json!({
            "msg_type": "ai_response",
            "text": response,
        })) {
            eprintln!("Failed to send AI response: {error:#}");
        }
    });
    session.send(&json!({ "msg_type": "committed_text" }))?;
    committed_text.clear();
    Ok(())
}
