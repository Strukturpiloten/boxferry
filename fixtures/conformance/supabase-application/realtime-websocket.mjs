export async function connectRealtimeWebSocket(
  endpoint,
  { createWebSocket = (url) => new WebSocket(url), timeoutMilliseconds = 30_000 } = {},
) {
  let websocket;
  try {
    websocket = createWebSocket(endpoint);
  } catch {
    throw new Error("Realtime WebSocket connection failed");
  }
  await new Promise((resolve, reject) => {
    let settled = false;
    const cleanup = () => {
      clearTimeout(timeout);
      websocket.removeEventListener("open", acceptConnection);
      websocket.removeEventListener("error", rejectSocketError);
      websocket.removeEventListener("close", rejectSocketClose);
    };
    const rejectConnection = (message) => {
      if (settled) return;
      settled = true;
      cleanup();
      websocket.close();
      reject(new Error(message));
    };
    const acceptConnection = () => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve();
    };
    const rejectSocketError = () => {
      rejectConnection("Realtime WebSocket connection failed");
    };
    const rejectSocketClose = () => {
      rejectConnection("Realtime WebSocket closed before connecting");
    };
    const timeout = setTimeout(() => {
      rejectConnection("Realtime WebSocket connection timed out");
    }, timeoutMilliseconds);

    websocket.addEventListener("open", acceptConnection, { once: true });
    websocket.addEventListener("error", rejectSocketError, { once: true });
    websocket.addEventListener("close", rejectSocketClose, { once: true });
  });
  return websocket;
}

function decodedMessage(event) {
  try {
    return JSON.parse(String(event.data));
  } catch {
    return null;
  }
}

export async function joinRealtimePostgresChanges(
  websocket,
  joinMessage,
  { timeoutMilliseconds = 30_000 } = {},
) {
  await new Promise((resolve, reject) => {
    let joined = false;
    let subscribed = false;
    let settled = false;
    const cleanup = () => {
      clearTimeout(timeout);
      websocket.removeEventListener("message", receiveMessage);
      websocket.removeEventListener("error", rejectSocketError);
      websocket.removeEventListener("close", rejectSocketClose);
    };
    const finish = (callback) => {
      if (settled) return;
      settled = true;
      cleanup();
      callback();
    };
    const rejectHandshake = (message) => {
      finish(() => reject(new Error(message)));
    };
    const acceptIfReady = () => {
      if (joined && subscribed) finish(resolve);
    };
    const receiveMessage = (event) => {
      const message = decodedMessage(event);
      if (
        message?.topic === joinMessage.topic &&
        message.event === "phx_reply" &&
        message.ref === joinMessage.ref
      ) {
        if (message.payload?.status !== "ok") {
          rejectHandshake("Realtime channel join failed");
          return;
        }
        joined = true;
        acceptIfReady();
      } else if (
        message?.topic === joinMessage.topic &&
        message.event === "system" &&
        message.payload?.extension === "postgres_changes"
      ) {
        if (message.payload?.status !== "ok") {
          rejectHandshake("Realtime PostgreSQL subscription failed");
          return;
        }
        subscribed = true;
        acceptIfReady();
      }
    };
    const rejectSocketError = () => {
      rejectHandshake("Realtime subscription handshake failed");
    };
    const rejectSocketClose = () => {
      rejectHandshake("Realtime WebSocket closed before subscription readiness");
    };
    const timeout = setTimeout(() => {
      rejectHandshake("Realtime subscription handshake timed out");
    }, timeoutMilliseconds);

    websocket.addEventListener("message", receiveMessage);
    websocket.addEventListener("error", rejectSocketError, { once: true });
    websocket.addEventListener("close", rejectSocketClose, { once: true });
    try {
      websocket.send(JSON.stringify(joinMessage));
    } catch {
      rejectHandshake("Realtime channel join send failed");
    }
  });
}

export function waitForRealtimeChange(
  websocket,
  topic,
  marker,
  { signal, timeoutMilliseconds = 30_000 } = {},
) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const cleanup = () => {
      clearTimeout(timeout);
      websocket.removeEventListener("message", receiveMessage);
      websocket.removeEventListener("error", rejectSocketError);
      websocket.removeEventListener("close", rejectSocketClose);
      signal?.removeEventListener("abort", rejectCancellation);
    };
    const finish = (callback) => {
      if (settled) return;
      settled = true;
      cleanup();
      callback();
    };
    const rejectWait = (message) => {
      finish(() => reject(new Error(message)));
    };
    const receiveMessage = (event) => {
      const message = decodedMessage(event);
      if (
        message?.topic === topic &&
        message.event === "postgres_changes" &&
        message.payload?.data?.record?.body === marker
      ) {
        finish(() => resolve(message));
      }
    };
    const rejectSocketError = () => {
      rejectWait("Realtime PostgreSQL change wait failed");
    };
    const rejectSocketClose = () => {
      rejectWait("Realtime WebSocket closed before PostgreSQL change");
    };
    const rejectCancellation = () => {
      rejectWait("Realtime PostgreSQL change wait cancelled");
    };
    const timeout = setTimeout(() => {
      rejectWait("Realtime PostgreSQL change wait timed out");
    }, timeoutMilliseconds);

    websocket.addEventListener("message", receiveMessage);
    websocket.addEventListener("error", rejectSocketError, { once: true });
    websocket.addEventListener("close", rejectSocketClose, { once: true });
    signal?.addEventListener("abort", rejectCancellation, { once: true });
    if (signal?.aborted) rejectCancellation();
  });
}
