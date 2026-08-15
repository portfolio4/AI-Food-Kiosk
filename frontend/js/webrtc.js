const RTCPeerConnectionImpl = globalThis.RTCPeerConnection
  ?? (await import("werift")).RTCPeerConnection;

export class LiveAudioRTCClient {
  constructor({ signalingUrl, iceServers, clientId }) {
    this.signalingUrl = signalingUrl;
    this.iceServers = iceServers;
    this.clientId = clientId;
    this.peerConnection = null;
    this.audioSender = null;
    this.connected = false;
    this.websocket = null;
  }

  async stream(audioTrack) {
    if (!this.connected) {
      await this.connect(audioTrack);
      return;
    }
    await this.#replaceTrack(audioTrack);
  }

  async connect(audioTrack) {
    if (!audioTrack) throw new Error("An audio track is required to connect");

    if (this.peerConnection) this.close();

    this.peerConnection = new RTCPeerConnectionImpl({
      iceServers: this.iceServers,
    });
    this.audioSender = this.peerConnection.addTrack(audioTrack);

    this.websocket = new WebSocket(this.signalingUrl);
    await new Promise((resolve, reject) => {
      this.websocket.onopen = resolve;
      this.websocket.onerror = () => reject(new Error("Signaling connection failed"));
    });

    const answerReceived = new Promise((resolve, reject) => {
      this.websocket.onmessage = async (event) => {
        try {
          await this.peerConnection.setRemoteDescription(
            JSON.parse(String(event.data)),
          );
          resolve();
        } catch (error) {
          reject(error);
        }
      };
    });

    const offer = await this.peerConnection.createOffer();
    await this.peerConnection.setLocalDescription(offer);
    await this.#waitForIceGathering();
    const description = this.peerConnection.localDescription.toJSON();
    this.websocket.send(JSON.stringify({
      ...description,
      client_id: String(this.clientId),
    }));

    await answerReceived;
    this.connected = true;
  }

  async pause() {
    await this.#replaceTrack(null);
  }

  async #replaceTrack(track) {
    if (!this.audioSender) {
      throw new Error("LiveAudioRTCClient is not connected");
    }
    await this.audioSender.replaceTrack(track);
  }

  close() {
    this.audioSender = null;
    this.connected = false;
    this.peerConnection?.close();
    this.websocket?.close();
    this.peerConnection = null;
    this.websocket = null;
  }

  async #waitForIceGathering() {
    if (this.peerConnection.iceGatheringState === "complete") return;

    // werift exposes a gatheringPromise; browsers use the standard state event.
    if (this.peerConnection.gatheringPromise) {
      await this.peerConnection.gatheringPromise;
      return;
    }

    await new Promise((resolve) => {
      const listener = () => {
        if (this.peerConnection.iceGatheringState === "complete") {
          this.peerConnection.removeEventListener(
            "icegatheringstatechange",
            listener,
          );
          resolve();
        }
      };
      this.peerConnection.addEventListener("icegatheringstatechange", listener);
    });
  }
}
