use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use agent::CodexAgent;
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc},
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Clone)]
pub struct RealtimeSession {
    client_id: String,
    sender: mpsc::UnboundedSender<Message>,
    agent: CodexAgent,
}

impl RealtimeSession {
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn agent(&self) -> &CodexAgent {
        &self.agent
    }

    pub fn send<T: Serialize>(&self, message: &T) -> Result<()> {
        let message =
            serde_json::to_string(message).context("failed to serialize realtime message")?;
        self.sender
            .send(Message::Text(message.into()))
            .context("realtime session is closed")
    }
}

#[derive(Clone, Default)]
pub struct RealtimeSessionsManager {
    sessions: Arc<Mutex<HashMap<String, RealtimeSession>>>,
}

impl RealtimeSessionsManager {
    pub async fn get(&self, client_id: &str) -> Option<RealtimeSession> {
        self.sessions.lock().await.get(client_id).cloned()
    }

    async fn insert(&self, session: RealtimeSession) {
        self.sessions
            .lock()
            .await
            .insert(session.client_id.clone(), session);
    }

    async fn remove(&self, client_id: &str, sender: &mpsc::UnboundedSender<Message>) {
        let mut sessions = self.sessions.lock().await;
        if sessions
            .get(client_id)
            .is_some_and(|session| session.sender.same_channel(sender))
        {
            sessions.remove(client_id);
        }
    }
}

type RealtimeMessageFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type RealtimeMessageHandler =
    Arc<dyn Fn(RealtimeSession, Value) -> RealtimeMessageFuture + Send + Sync>;

pub struct RealtimeServer {
    address: String,
    sessions: RealtimeSessionsManager,
    initial_agent_prompt: String,
    message_handler: RealtimeMessageHandler,
}

impl RealtimeServer {
    pub fn new<H, F>(
        address: impl Into<String>,
        initial_agent_prompt: impl Into<String>,
        message_handler: H,
    ) -> Self
    where
        H: Fn(RealtimeSession, Value) -> F + Send + Sync + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        Self {
            address: address.into(),
            sessions: RealtimeSessionsManager::default(),
            initial_agent_prompt: initial_agent_prompt.into(),
            message_handler: Arc::new(move |session, message| {
                Box::pin(message_handler(session, message))
            }),
        }
    }

    pub fn sessions(&self) -> RealtimeSessionsManager {
        self.sessions.clone()
    }

    pub async fn run(self) -> Result<()> {
        let listener = TcpListener::bind(&self.address).await?;
        println!(
            "Realtime server listening on ws://{}/realtime",
            self.address
        );

        loop {
            let (stream, address) = listener.accept().await?;
            let sessions = self.sessions.clone();
            let message_handler = Arc::clone(&self.message_handler);
            let initial_agent_prompt = self.initial_agent_prompt.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    handle_connection(stream, sessions, message_handler, initial_agent_prompt).await
                {
                    eprintln!("Realtime connection from {address} failed: {error:#}");
                }
            });
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    sessions: RealtimeSessionsManager,
    message_handler: RealtimeMessageHandler,
    initial_agent_prompt: String,
) -> Result<()> {
    let websocket = accept_async(stream)
        .await
        .context("Realtime WebSocket handshake failed")?;
    let (mut websocket_sink, mut websocket_stream) = websocket.split();
    let login_message = websocket_stream
        .next()
        .await
        .context("client disconnected before login")??;
    let login: Value = serde_json::from_str(
        login_message
            .to_text()
            .context("expected login as a text message")?,
    )
    .context("invalid realtime login JSON")?;
    if login.get("msg_type").and_then(Value::as_str) != Some("login") {
        bail!("first realtime message must be a login");
    }
    let client_id = login
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .context("login requires a non-empty client_id")?
        .to_owned();

    let agent = CodexAgent::new(initial_agent_prompt)
        .await
        .context("failed to start Codex agent for realtime session")?;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let session = RealtimeSession {
        client_id: client_id.clone(),
        sender: sender.clone(),
        agent,
    };
    sessions.insert(session.clone()).await;
    websocket_sink
        .send(Message::Text(r#"{"msg_type":"success"}"#.into()))
        .await?;

    let connection_result: Result<()> = loop {
        tokio::select! {
            outbound = receiver.recv() => {
                match outbound {
                    Some(message) => {
                        if let Err(error) = websocket_sink.send(message).await {
                            break Err(error.into());
                        }
                    }
                    None => break Ok(()),
                }
            }
            inbound = websocket_stream.next() => {
                match inbound {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str(&text) {
                            Ok(message) => message_handler(session.clone(), message).await,
                            Err(error) => eprintln!(
                                "Ignoring invalid realtime message from {client_id}: {error}",
                            ),
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(error) = websocket_sink.send(Message::Pong(payload)).await {
                            break Err(error.into());
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break Ok(()),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => break Err(error.into()),
                }
            }
        }
    };

    sessions.remove(&client_id, &sender).await;
    connection_result
}
