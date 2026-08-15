import { RealtimeClient } from "./realtime.js";
import { BrowserSpeaker } from "./speak.js";
import { LiveAudioRTCClient } from "./webrtc.js";

const WELCOME_MESSAGE = "Hi there! What can I get started for you today?";

const elements = {
  recordButton: document.querySelector("#recordBtn"),
  recordIcon: document.querySelector("#recordIcon"),
  recordText: document.querySelector("#recordText"),
  recordingOverlay: document.querySelector("#recordingOverlay"),
  conversation: document.querySelector("#chatContainerWrapper"),
  chatHistory: document.querySelector("#chatHistory"),
  speechEnabled: document.querySelector("#speechEnabled"),
};

// Once accounts exist, use the user's unique account ID instead.
const clientId = crypto.randomUUID();
const realtimeClient = new RealtimeClient({
  url: "ws://127.0.0.1:9002/realtime",
  clientId: clientId,
});
const speaker = new BrowserSpeaker();
const rtcClient = new LiveAudioRTCClient({
  signalingUrl: "ws://127.0.0.1:9001",
  clientId: clientId,
  iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
});

let isRecording = false;
let microphoneStream = null;
let typingIndicator = null;
let microphoneTrack = null;
let activeUserMessage = null;
let activeUserFragments = [];
let assistantSpeechQueue = [];
let assistantIsSpeaking = false;
let speechGeneration = 0;

function scrollChatToLatest() {
  elements.chatHistory.scrollTop = elements.chatHistory.scrollHeight;
}

function createTypingIndicator() {
  const item = document.createElement("li");
  item.className = "mt-sm flex max-w-[85%] flex-col items-end self-end";
  item.setAttribute("aria-label", "Listening to your order");

  const label = document.createElement("span");
  label.className = "mb-xs mr-sm font-label-bold text-label-bold text-on-surface-variant";
  label.textContent = "You";

  const bubble = document.createElement("span");
  bubble.className = "flex min-h-[44px] items-center rounded-xl rounded-tr-none bg-primary-container p-sm text-on-primary-container";
  bubble.setAttribute("aria-hidden", "true");

  const delayClasses = ["[animation-delay:-.32s]", "[animation-delay:-.16s]", "[animation-delay:0s]"];
  delayClasses.forEach((delayClass) => {
    const dot = document.createElement("span");
    dot.className = `mr-xs inline-block h-2 w-2 rounded-full bg-on-primary-container motion-safe:animate-typing ${delayClass}`;
    bubble.append(dot);
  });

  item.append(label, bubble);
  return item;
}

function createMessage(text, isUser = false) {
  const item = document.createElement("li");
  item.className = `mt-sm flex max-w-[85%] flex-col opacity-0 transition-opacity duration-500 ${
    isUser ? "items-end self-end" : "items-start"
  }`;

  const label = document.createElement("span");
  label.className = `mb-xs font-label-bold text-label-bold text-on-surface-variant ${isUser ? "mr-sm" : "ml-sm"}`;
  label.textContent = isUser ? "You" : "Assistant";

  const message = document.createElement("p");
  message.className = `rounded-xl p-sm font-body-lg text-body-lg ${
    isUser
      ? "rounded-tr-none bg-primary-container text-on-primary-container"
      : "rounded-tl-none bg-surface-variant text-on-surface-variant"
  }`;
  message.textContent = text;

  item.append(label, message);
  requestAnimationFrame(() => item.classList.remove("opacity-0"));
  return item;
}

function appendBufferedCommittedText(text) {
  typingIndicator?.remove();
  typingIndicator = null;

  if (!activeUserMessage) {
    activeUserMessage = createMessage("", true);
    const bubble = activeUserMessage.querySelector("p");
    bubble.classList.add("motion-safe:animate-pulse");
    elements.chatHistory.append(activeUserMessage);
  }

  activeUserFragments.push(text);
  const bubble = activeUserMessage.querySelector("p");
  bubble.textContent = activeUserFragments.join(" ") + "...";
  scrollChatToLatest();
}

function commitActiveUserMessage() {
  if (!activeUserMessage) return;

  const bubble = activeUserMessage.querySelector("p");
  bubble.textContent = activeUserFragments.join(" ");
  bubble.classList.remove("motion-safe:animate-pulse");
  activeUserMessage = null;
  activeUserFragments = [];
  void speakNextAssistantResponse();
}

function appendAssistantResponse(text) {
  const message = createMessage(text);
  const pendingUserMessage = activeUserMessage ?? typingIndicator;

  if (pendingUserMessage?.parentNode === elements.chatHistory) {
    elements.chatHistory.insertBefore(message, pendingUserMessage);
  } else {
    elements.chatHistory.append(message);
  }

  scrollChatToLatest();
  queueAssistantResponse(text);
}

function handleRealtimeMessage(message) {
  if (message.msg_type === "buffered_committed_text" && typeof message.text === "string") {
    appendBufferedCommittedText(message.text);
  } else if (message.msg_type === "committed_text") {
    commitActiveUserMessage();
  } else if (message.msg_type === "ai_response" && typeof message.text === "string") {
    appendAssistantResponse(message.text);
  }
}

function queueAssistantResponse(text) {
  if (!elements.speechEnabled.checked) return;

  assistantSpeechQueue.push(text);
  void speakNextAssistantResponse();
}

async function speakNextAssistantResponse() {
  if (assistantIsSpeaking || activeUserMessage || typingIndicator) return;
  if (!elements.speechEnabled.checked) {
    assistantSpeechQueue = [];
    return;
  }

  const text = assistantSpeechQueue.shift();
  if (!text) return;

  assistantIsSpeaking = true;
  const generation = speechGeneration;
  if (microphoneTrack) microphoneTrack.enabled = false;

  try {
    await speaker.speak(text);
  } catch (error) {
    console.error("Browser speech playback failed:", error);
  } finally {
    if (generation !== speechGeneration) return;
    assistantIsSpeaking = false;
    if (isRecording && microphoneTrack) microphoneTrack.enabled = true;
    void speakNextAssistantResponse();
  }
}

function renderRecordingState(recording) {
  elements.recordButton.setAttribute("aria-pressed", String(recording));
  elements.recordButton.classList.toggle("bg-primary", !recording);
  elements.recordButton.classList.toggle("hover:bg-surface-tint", !recording);
  elements.recordButton.classList.toggle("bg-error", recording);
  elements.recordButton.classList.toggle("hover:bg-on-error-container", recording);
  elements.recordIcon.textContent = recording ? "stop" : "mic";
  elements.recordText.textContent = recording ? "Done Ordering" : "Tap to Order";
  elements.recordingOverlay.classList.toggle("opacity-0", !recording);
  elements.conversation.classList.toggle("border-primary", recording);
}

function showRecordingError(error) {
  console.error("Microphone streaming failed:", error);
  const message = error?.name === "NotAllowedError"
    ? "Microphone access was denied. Please allow microphone access and try again."
    : "I could not connect to the ordering service. Please try again.";
  elements.chatHistory.append(createMessage(message));
  scrollChatToLatest();
}

async function stopRecording() {
  elements.recordButton.disabled = true;
  try {
    await rtcClient.pause();
    if (microphoneTrack) microphoneTrack.enabled = false;
    isRecording = false;
    renderRecordingState(false);
    typingIndicator?.remove();
    typingIndicator = null;
    void speakNextAssistantResponse();
  } catch (error) {
    showRecordingError(error);
  } finally {
    elements.recordButton.disabled = false;
  }
}

function startNextCustomer() {
  elements.chatHistory.replaceChildren();
  typingIndicator = null;
  activeUserMessage = null;
  activeUserFragments = [];
  assistantSpeechQueue = [];
  speechGeneration += 1;
  window.speechSynthesis.cancel();
  assistantIsSpeaking = false;

  realtimeClient.send({ msg_type: "next_customer" });
  appendAssistantResponse(WELCOME_MESSAGE);
}

async function startRecording() {
  elements.recordButton.disabled = true;
  elements.recordText.textContent = "Connecting...";

  try {
    await realtimeClient.ready;
    startNextCustomer();
    if (!navigator.mediaDevices?.getUserMedia) {
      throw new Error("Microphone recording is not supported by this browser");
    }

    if (!microphoneTrack) {
      microphoneStream = await navigator.mediaDevices.getUserMedia({
        audio: {
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
      [microphoneTrack] = microphoneStream.getAudioTracks();
      if (!microphoneTrack) throw new Error("No microphone audio track is available");
    }

    microphoneTrack.enabled = !assistantIsSpeaking;
    await rtcClient.stream(microphoneTrack);

    isRecording = true;
    microphoneTrack.enabled = !assistantIsSpeaking;
    renderRecordingState(true);
    typingIndicator?.remove();
    typingIndicator = createTypingIndicator();
    elements.chatHistory.append(typingIndicator);
    scrollChatToLatest();
  } catch (error) {
    if (microphoneTrack) microphoneTrack.enabled = false;
    isRecording = false;
    renderRecordingState(false);
    showRecordingError(error);
  } finally {
    elements.recordButton.disabled = false;
  }
}

async function toggleRecording() {
  if (isRecording) {
    await stopRecording();
    return;
  }

  await startRecording();
}

// Uses native dialogs while also supporting close buttons and backdrop clicks.
function initializeDialogs() {
  document.querySelectorAll("[data-dialog-target]").forEach((button) => {
    button.addEventListener("click", () => {
      document.getElementById(button.dataset.dialogTarget)?.showModal();
    });
  });

  document.querySelectorAll("dialog").forEach((dialog) => {
    dialog.querySelector("[data-dialog-close]")?.addEventListener("click", () => dialog.close());
    dialog.addEventListener("click", (event) => {
      if (event.target === dialog) dialog.close();
    });
  });
}

// Mobile navigation moves focus to the chosen workspace panel.
function initializeMobileNavigation() {
  document.querySelectorAll("[data-mobile-action]").forEach((button) => {
    button.addEventListener("click", () => {
      const target = document.getElementById(button.dataset.mobileAction);
      target?.scrollIntoView({ behavior: "smooth", block: "start" });
      target?.querySelector("button, a, [tabindex]")?.focus({ preventScroll: true });

      document.querySelectorAll("[data-mobile-action]").forEach((navButton) => {
        const active = navButton === button;
        navButton.toggleAttribute("aria-current", active);
        navButton.classList.toggle("rounded-full", active);
        navButton.classList.toggle("bg-primary-container", active);
        navButton.classList.toggle("px-6", active);
        navButton.classList.toggle("py-2", active);
        navButton.classList.toggle("text-on-primary-container", active);
      });
    });
  });
}

realtimeClient.onmessage = handleRealtimeMessage;

elements.recordButton.addEventListener("click", () => void toggleRecording());
window.addEventListener("beforeunload", () => {
  microphoneTrack?.stop();
  microphoneStream?.getTracks().forEach((track) => track.stop());
  rtcClient.close();
  realtimeClient.close();
});
initializeDialogs();
initializeMobileNavigation();
