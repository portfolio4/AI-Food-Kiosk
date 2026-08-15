use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use realtime::{RealtimeSession, RealtimeSessionsManager};
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use webrtc::{
    api::{APIBuilder, media_engine::MediaEngine},
    peer_connection::{
        configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription,
    },
    track::track_remote::TrackRemote,
};

pub type AudioTrack = Arc<TrackRemote>;

type AudioTrackFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type AudioTrackHandler =
    Arc<dyn Fn(AudioTrack, RealtimeSession) -> AudioTrackFuture + Send + Sync>;

pub struct LiveAudioRTCServer {
    signaling_address: String,
    realtime_sessions: RealtimeSessionsManager,
    audio_track_handler: AudioTrackHandler,
}

impl LiveAudioRTCServer {
    pub fn new<H, F>(
        signaling_address: impl Into<String>,
        realtime_sessions: RealtimeSessionsManager,
        audio_track_handler: H,
    ) -> Self
    where
        H: Fn(AudioTrack, RealtimeSession) -> F + Send + Sync + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        Self {
            signaling_address: signaling_address.into(),
            realtime_sessions,
            audio_track_handler: Arc::new(move |track, session| {
                Box::pin(audio_track_handler(track, session))
            }),
        }
    }

    pub async fn run(self) -> Result<()> {
        let listener = TcpListener::bind(&self.signaling_address).await?;
        println!(
            "WebRTC signaling server listening on ws://{}",
            self.signaling_address
        );

        loop {
            let (stream, address) = listener.accept().await?;
            let audio_track_handler = Arc::clone(&self.audio_track_handler);
            let realtime_sessions = self.realtime_sessions.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    handle_connection(stream, audio_track_handler, realtime_sessions).await
                {
                    eprintln!("Signaling connection from {address} failed: {error:#}");
                }
            });
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    audio_track_handler: AudioTrackHandler,
    realtime_sessions: RealtimeSessionsManager,
) -> Result<()> {
    let mut websocket = accept_async(stream)
        .await
        .context("WebSocket handshake failed")?;
    let offer_message = websocket
        .next()
        .await
        .context("client disconnected before sending an offer")??;
    let offer_json: Value = serde_json::from_str(
        offer_message
            .to_text()
            .context("expected the SDP offer as a text message")?,
    )
    .context("invalid signaling JSON")?;
    let client_id = offer_json
        .get("client_id")
        .and_then(Value::as_str)
        .context("signaling offer requires client_id")?;
    let realtime_session = realtime_sessions
        .get(client_id)
        .await
        .context("client_id does not have a logged-in realtime session")?;
    let offer: RTCSessionDescription =
        serde_json::from_value(offer_json).context("invalid WebRTC offer")?;

    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs()?;
    let api = APIBuilder::new().with_media_engine(media_engine).build();
    let peer_connection = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    peer_connection.on_track(Box::new(move |track, _, _| {
        audio_track_handler(track, realtime_session.clone())
    }));

    peer_connection.set_remote_description(offer).await?;
    let answer = peer_connection.create_answer(None).await?;
    let mut gathering_complete = peer_connection.gathering_complete_promise().await;
    peer_connection.set_local_description(answer).await?;
    let _ = gathering_complete.recv().await;

    let local_description = peer_connection
        .local_description()
        .await
        .context("peer connection did not produce a local description")?;
    websocket
        .send(Message::Text(
            serde_json::to_string(&local_description)?.into(),
        ))
        .await?;

    while let Some(message) = websocket.next().await {
        match message? {
            Message::Close(_) => break,
            Message::Ping(payload) => websocket.send(Message::Pong(payload)).await?,
            _ => {}
        }
    }

    peer_connection.close().await?;
    Ok(())
}
