export class RealtimeClient {
  constructor({ url, clientId }) {
    if (!url) throw new Error("A realtime WebSocket URL is required");
    if (!clientId) throw new Error("A client ID is required");

    this.onmessage = null;
    this.websocket = new WebSocket(url);
    this.ready = new Promise((resolve, reject) => {
      let loggedIn = false;

      this.websocket.onopen = () => {
        this.websocket.send(JSON.stringify({
          msg_type: "login",
          client_id: String(clientId),
        }));
      };
      this.websocket.onerror = () => {
        if (!loggedIn) reject(new Error("Realtime connection failed"));
      };
      this.websocket.onclose = () => {
        if (!loggedIn) reject(new Error("Realtime connection closed before login"));
      };
      this.websocket.onmessage = (event) => {
        try {
          const message = JSON.parse(String(event.data));
          if (!loggedIn && message.msg_type === "success") {
            loggedIn = true;
            resolve();
          }
          this.onmessage?.(message);
        } catch (error) {
          if (!loggedIn) reject(error);
          else console.error("Invalid realtime server message:", error);
        }
      };
    });
  }

  send(message) {
    if (this.websocket.readyState !== WebSocket.OPEN) {
      throw new Error("RealtimeClient is not connected");
    }
    this.websocket.send(
      typeof message === "string" ? message : JSON.stringify(message),
    );
  }

  close() {
    this.websocket.close();
  }
}
