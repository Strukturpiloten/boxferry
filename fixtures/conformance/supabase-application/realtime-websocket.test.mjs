import assert from "node:assert/strict";
import { inspect } from "node:util";
import test from "node:test";

import {
  connectRealtimeWebSocket,
  joinRealtimePostgresChanges,
  waitForRealtimeChange,
} from "./realtime-websocket.mjs";

const realtimeTopic = "realtime:public:boxferry_items";

class TestSocket extends EventTarget {
  closed = false;
  sent = [];
  sendFailure;

  constructor(objectSentinel) {
    super();
    this.objectSentinel = objectSentinel;
  }

  close() {
    this.closed = true;
  }

  send(message) {
    if (this.sendFailure) throw new Error(this.sendFailure);
    this.sent.push(message);
  }
}

function emitMessage(socket, message, topic = realtimeTopic) {
  const event = new Event("message");
  Object.defineProperty(event, "data", { value: JSON.stringify({ topic, ...message }) });
  socket.dispatchEvent(event);
}

function joinMessage(credential) {
  return {
    topic: realtimeTopic,
    event: "phx_join",
    ref: "1",
    payload: {
      access_token: credential,
      config: {
        postgres_changes: [{ event: "INSERT", schema: "public", table: "boxferry_items" }],
      },
    },
  };
}

function assertCredentialFree(error, endpoint, credential, socket) {
  const rendered = `${String(error)}\n${error.stack ?? ""}\n${inspect(error)}`;
  const queryName = ["api", "key"].join("");
  for (const forbidden of [credential, endpoint, `${queryName}=`, socket.objectSentinel]) {
    assert.equal(rendered.includes(forbidden), false, `error exposed ${forbidden}`);
  }
  assert.equal(error.cause, undefined);
  assert.equal(Object.hasOwn(error, "target"), false);
  return true;
}

test("constructor failures cannot expose the credential-bearing endpoint", async () => {
  const credential = ["private", "constructor", "sentinel"].join("-");
  const queryName = ["api", "key"].join("");
  const endpoint = `ws://example.invalid/realtime?${queryName}=${credential}`;
  const socket = new TestSocket(["constructor", "socket", "sentinel"].join("-"));

  await assert.rejects(
    connectRealtimeWebSocket(endpoint, {
      createWebSocket: () => {
        throw new Error(`invalid WebSocket ${endpoint}`);
      },
    }),
    (error) => {
      assert.equal(error.message, "Realtime WebSocket connection failed");
      return assertCredentialFree(error, endpoint, credential, socket);
    },
  );
});

test("connection errors are bounded and credential-free", async () => {
  const credential = ["private", "credential", "sentinel"].join("-");
  const queryName = ["api", "key"].join("");
  const endpoint = `ws://example.invalid/realtime?${queryName}=${credential}`;
  const socket = new TestSocket(["socket", "object", "sentinel"].join("-"));
  const started = Date.now();
  const connection = connectRealtimeWebSocket(endpoint, {
    createWebSocket: () => socket,
    timeoutMilliseconds: 1_000,
  });
  queueMicrotask(() => socket.dispatchEvent(new Event("error")));

  await assert.rejects(connection, (error) => {
    assert.equal(error.message, "Realtime WebSocket connection failed");
    return assertCredentialFree(error, endpoint, credential, socket);
  });
  assert.equal(socket.closed, true);
  assert.ok(Date.now() - started < 1_000, "connection error was not bounded");
});

test("connection timeouts close the socket without exposing credentials", async () => {
  const credential = ["private", "timeout", "sentinel"].join("-");
  const queryName = ["api", "key"].join("");
  const endpoint = `ws://example.invalid/realtime?${queryName}=${credential}`;
  const socket = new TestSocket(["timeout", "socket", "sentinel"].join("-"));
  const started = Date.now();

  await assert.rejects(
    connectRealtimeWebSocket(endpoint, {
      createWebSocket: () => socket,
      timeoutMilliseconds: 10,
    }),
    (error) => {
      assert.equal(error.message, "Realtime WebSocket connection timed out");
      return assertCredentialFree(error, endpoint, credential, socket);
    },
  );
  assert.equal(socket.closed, true);
  assert.ok(Date.now() - started < 1_000, "connection timeout was not bounded");
});

test("connection close is rejected before the open deadline", async () => {
  const credential = ["private", "close", "sentinel"].join("-");
  const endpoint = `ws://example.invalid/realtime?apikey=${credential}`;
  const socket = new TestSocket(["close", "socket", "sentinel"].join("-"));
  const connection = connectRealtimeWebSocket(endpoint, {
    createWebSocket: () => socket,
    timeoutMilliseconds: 1_000,
  });
  queueMicrotask(() => socket.dispatchEvent(new Event("close")));

  await assert.rejects(connection, (error) => {
    assert.equal(error.message, "Realtime WebSocket closed before connecting");
    return assertCredentialFree(error, endpoint, credential, socket);
  });
  assert.equal(socket.closed, true);
});

test("subscription handshake requires successful join and PostgreSQL acknowledgement", async () => {
  const credential = ["private", "success", "sentinel"].join("-");
  const socket = new TestSocket(["handshake", "socket", "sentinel"].join("-"));
  const handshake = joinRealtimePostgresChanges(socket, joinMessage(credential), {
    timeoutMilliseconds: 1_000,
  });
  let resolved = false;
  void handshake.then(() => {
    resolved = true;
  });

  emitMessage(
    socket,
    { event: "phx_reply", ref: "1", payload: { status: "ok" } },
    "realtime:unrelated",
  );
  emitMessage(
    socket,
    { event: "system", payload: { extension: "postgres_changes", status: "ok" } },
    "realtime:unrelated",
  );
  await Promise.resolve();
  assert.equal(resolved, false);

  emitMessage(socket, {
    event: "system",
    payload: { extension: "postgres_changes", status: "ok" },
  });
  await Promise.resolve();
  assert.equal(resolved, false);
  emitMessage(socket, { event: "phx_reply", ref: "1", payload: { status: "ok" } });
  await handshake;
  assert.equal(resolved, true);
  assert.deepEqual(socket.sent.map(JSON.parse), [joinMessage(credential)]);
});

test("join rejection is immediate, credential-free, and leaves no pending subscription wait", async () => {
  const credential = ["private", "join", "sentinel"].join("-");
  const payload = joinMessage(credential);
  const renderedPayload = JSON.stringify(payload);
  const socket = new TestSocket(["join", "socket", "sentinel"].join("-"));
  const handshake = joinRealtimePostgresChanges(socket, payload, { timeoutMilliseconds: 1_000 });
  emitMessage(socket, { event: "phx_reply", ref: "1", payload: { status: "error" } });

  await assert.rejects(handshake, (error) => {
    assert.equal(error.message, "Realtime channel join failed");
    return assertCredentialFree(error, renderedPayload, credential, socket);
  });
  emitMessage(socket, {
    event: "system",
    payload: { extension: "postgres_changes", status: "error" },
  });
  socket.dispatchEvent(new Event("error"));
  await Promise.resolve();
});

test("subscription rejection is immediate and credential-free", async () => {
  const credential = ["private", "subscription", "sentinel"].join("-");
  const payload = joinMessage(credential);
  const socket = new TestSocket(["subscription", "socket", "sentinel"].join("-"));
  const handshake = joinRealtimePostgresChanges(socket, payload, { timeoutMilliseconds: 1_000 });
  emitMessage(socket, { event: "phx_reply", ref: "1", payload: { status: "ok" } });
  emitMessage(socket, {
    event: "system",
    payload: { extension: "postgres_changes", status: "error" },
  });

  await assert.rejects(handshake, (error) => {
    assert.equal(error.message, "Realtime PostgreSQL subscription failed");
    return assertCredentialFree(error, JSON.stringify(payload), credential, socket);
  });
});

test("close before subscription acknowledgement rejects immediately", async () => {
  const credential = ["private", "early-close", "sentinel"].join("-");
  const payload = joinMessage(credential);
  const socket = new TestSocket(["early", "close", "socket"].join("-"));
  const handshake = joinRealtimePostgresChanges(socket, payload, { timeoutMilliseconds: 1_000 });
  emitMessage(socket, { event: "phx_reply", ref: "1", payload: { status: "ok" } });
  socket.dispatchEvent(new Event("close"));

  await assert.rejects(handshake, (error) => {
    assert.equal(error.message, "Realtime WebSocket closed before subscription readiness");
    return assertCredentialFree(error, JSON.stringify(payload), credential, socket);
  });
});

test("missing subscription acknowledgement has one bounded credential-free timeout", async () => {
  const credential = ["private", "handshake-timeout", "sentinel"].join("-");
  const payload = joinMessage(credential);
  const socket = new TestSocket(["handshake", "timeout", "socket"].join("-"));
  const handshake = joinRealtimePostgresChanges(socket, payload, { timeoutMilliseconds: 10 });
  emitMessage(socket, { event: "phx_reply", ref: "1", payload: { status: "ok" } });

  await assert.rejects(handshake, (error) => {
    assert.equal(error.message, "Realtime subscription handshake timed out");
    return assertCredentialFree(error, JSON.stringify(payload), credential, socket);
  });
});

test("join send failure cannot expose the credential-bearing payload", async () => {
  const credential = ["private", "send", "sentinel"].join("-");
  const payload = joinMessage(credential);
  const socket = new TestSocket(["send", "socket", "sentinel"].join("-"));
  socket.sendFailure = `cannot send ${JSON.stringify(payload)}`;

  await assert.rejects(
    joinRealtimePostgresChanges(socket, payload, { timeoutMilliseconds: 1_000 }),
    (error) => {
      assert.equal(error.message, "Realtime channel join send failed");
      return assertCredentialFree(error, JSON.stringify(payload), credential, socket);
    },
  );
});

test("change wait handles close and caller cancellation without exposing marker data", async () => {
  const marker = ["private", "change", "sentinel"].join("-");
  const socket = new TestSocket(["change", "socket", "sentinel"].join("-"));
  const closeWait = waitForRealtimeChange(socket, realtimeTopic, marker, { timeoutMilliseconds: 1_000 });
  socket.dispatchEvent(new Event("close"));
  await assert.rejects(closeWait, (error) => {
    assert.equal(error.message, "Realtime WebSocket closed before PostgreSQL change");
    return assertCredentialFree(error, marker, marker, socket);
  });

  const controller = new AbortController();
  const cancelledWait = waitForRealtimeChange(socket, realtimeTopic, marker, {
    signal: controller.signal,
    timeoutMilliseconds: 1_000,
  });
  controller.abort();
  await assert.rejects(cancelledWait, (error) => {
    assert.equal(error.message, "Realtime PostgreSQL change wait cancelled");
    return assertCredentialFree(error, marker, marker, socket);
  });
});

test("change wait accepts only the joined topic", async () => {
  const marker = "boxferry-topic-marker";
  const socket = new TestSocket("topic-socket");
  const change = waitForRealtimeChange(socket, realtimeTopic, marker, {
    timeoutMilliseconds: 1_000,
  });
  let resolved = false;
  void change.then(() => {
    resolved = true;
  });

  const message = {
    event: "postgres_changes",
    payload: { data: { record: { body: marker } } },
  };
  emitMessage(socket, message, "realtime:unrelated");
  await Promise.resolve();
  assert.equal(resolved, false);

  emitMessage(socket, message);
  const accepted = await change;
  assert.equal(accepted.topic, realtimeTopic);
});
