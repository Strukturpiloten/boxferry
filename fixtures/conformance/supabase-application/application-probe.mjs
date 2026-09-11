import assert from "node:assert/strict";

const phase = process.argv[2];
assert.match(phase ?? "", /^(seed|verify)$/);

function required(name) {
  const value = process.env[name];
  assert.ok(value, `${name} must be set`);
  return value;
}

const base = process.env.BF_SUPABASE_URL ?? "http://kong:8000";
const realtimeBase = process.env.BF_REALTIME_URL ?? "http://realtime:4000";
const supavisorBase = process.env.BF_SUPAVISOR_URL ?? "http://supavisor:4000";
const anonKey = required("BF_ANON_KEY");
const serviceKey = required("BF_SERVICE_KEY");
const email = required("BF_TEST_EMAIL");
const password = required("BF_TEST_PASSWORD");
const objectBody = "boxferry-supabase-storage-round-trip\n";

const anonHeaders = {
  apikey: anonKey,
  authorization: `Bearer ${anonKey}`,
};
const serviceHeaders = {
  apikey: serviceKey,
  authorization: `Bearer ${serviceKey}`,
};

async function request(path, options = {}, accepted = [200]) {
  const response = await fetch(`${base}${path}`, options);
  assert.ok(
    accepted.includes(response.status),
    `${options.method ?? "GET"} ${path} returned ${response.status}`,
  );
  return response;
}

async function jsonRequest(path, options = {}, accepted = [200]) {
  const response = await request(path, options, accepted);
  const text = await response.text();
  return text ? JSON.parse(text) : {};
}

async function signUp() {
  const body = await jsonRequest("/auth/v1/signup", {
    method: "POST",
    headers: { ...anonHeaders, "content-type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  assert.equal(typeof body.access_token, "string");
  return body.access_token;
}

async function signIn() {
  const body = await jsonRequest("/auth/v1/token?grant_type=password", {
    method: "POST",
    headers: { ...anonHeaders, "content-type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  assert.equal(typeof body.access_token, "string");
  return body.access_token;
}

async function assertCurrentUser(token) {
  const body = await jsonRequest("/auth/v1/user", {
    headers: { apikey: anonKey, authorization: `Bearer ${token}` },
  });
  assert.equal(body.email, email);
}

function waitForSocket(socket, predicate, description) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error(`timed out waiting for ${description}`)),
      30_000,
    );
    const listener = (event) => {
      let message;
      try {
        message = JSON.parse(String(event.data));
      } catch {
        return;
      }
      if (predicate(message)) {
        clearTimeout(timeout);
        socket.removeEventListener("message", listener);
        resolve(message);
      }
    };
    socket.addEventListener("message", listener);
  });
}

async function realtimeInsert(marker) {
  const websocket = new WebSocket(
    `${base.replace(/^http/, "ws")}/realtime/v1/websocket?apikey=${encodeURIComponent(anonKey)}&vsn=1.0.0`,
  );
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error("realtime open timeout")), 30_000);
    websocket.addEventListener(
      "open",
      () => {
        clearTimeout(timeout);
        resolve();
      },
      { once: true },
    );
    websocket.addEventListener("error", reject, { once: true });
  });

  const joinReply = waitForSocket(
    websocket,
    (message) => message.event === "phx_reply" && message.ref === "1",
    "Realtime channel join",
  );
  websocket.send(
    JSON.stringify({
      topic: "realtime:public:boxferry_items",
      event: "phx_join",
      ref: "1",
      payload: {
        access_token: anonKey,
        config: {
          broadcast: { ack: false, self: false },
          presence: { key: "" },
          postgres_changes: [{ event: "INSERT", schema: "public", table: "boxferry_items" }],
        },
      },
    }),
  );
  const joined = await joinReply;
  assert.equal(joined.payload?.status, "ok");

  const change = waitForSocket(
    websocket,
    (message) =>
      message.event === "postgres_changes" && message.payload?.data?.record?.body === marker,
    "Realtime PostgreSQL change",
  );
  const inserted = await jsonRequest(
    "/rest/v1/boxferry_items",
    {
      method: "POST",
      headers: {
        ...serviceHeaders,
        "content-type": "application/json",
        prefer: "return=representation",
      },
      body: JSON.stringify({ body: marker }),
    },
    [200, 201],
  );
  assert.equal(inserted[0]?.body, marker);
  await change;
  websocket.close();
}

async function assertRows(expectedMarkers) {
  const rows = await jsonRequest("/rest/v1/boxferry_items?select=body&order=body.asc", {
    headers: serviceHeaders,
  });
  assert.deepEqual(
    rows.map((row) => row.body),
    [...expectedMarkers].sort(),
  );
}

async function seedStorage() {
  await jsonRequest(
    "/storage/v1/bucket",
    {
      method: "POST",
      headers: { ...serviceHeaders, "content-type": "application/json" },
      body: JSON.stringify({ id: "boxferry", name: "boxferry", public: false }),
    },
    [200, 201, 409],
  );
  await request(
    "/storage/v1/object/boxferry/probe.txt",
    {
      method: "POST",
      headers: {
        ...serviceHeaders,
        "content-type": "text/plain",
        "x-upsert": "true",
      },
      body: objectBody,
    },
    [200, 201],
  );
}

async function assertStorage() {
  const response = await request("/storage/v1/object/authenticated/boxferry/probe.txt", {
    headers: serviceHeaders,
  });
  assert.equal(await response.text(), objectBody);
}

async function assertEdgeFunction() {
  const edgeInputs = {
    seed: {
      body: "boxferry-supabase-edge-seed",
      checksum: "cd2c400852048a021086994cc5f266472d53e72ffddb1c1f5d01a17ddaa27ca4",
    },
    verify: {
      body: "boxferry-supabase-edge-verify",
      checksum: "71059a67ee64b2891c41a31b66660b18e366342f765ccf07fd68fb0436eb6638",
    },
  };
  const expected = edgeInputs[phase];
  const body = await jsonRequest("/functions/v1/main", {
    method: "POST",
    headers: { ...anonHeaders, "content-type": "text/plain" },
    body: expected.body,
  });
  assert.equal(body.application, "boxferry-supabase");
  assert.equal(body.checksum, expected.checksum);
}

async function assertServiceGraph() {
  await request("/studio/api/platform/profile");
  await request("/pg/health");

  const realtime = await fetch(`${realtimeBase}/api/tenants/realtime-dev/health`, {
    headers: { authorization: `Bearer ${anonKey}` },
  });
  assert.ok(realtime.ok, `Realtime health returned ${realtime.status}`);

  const supavisor = await fetch(`${supavisorBase}/api/health`);
  assert.ok(supavisor.ok, `Supavisor health returned ${supavisor.status}`);
}

const seedMarker = "boxferry-realtime-seed";
const persistenceMarker = "boxferry-realtime-after-recreate";
const accessToken = phase === "seed" ? await signUp() : await signIn();
await assertCurrentUser(accessToken);

if (phase === "seed") {
  await realtimeInsert(seedMarker);
  await seedStorage();
  await assertRows([seedMarker]);
} else {
  await realtimeInsert(persistenceMarker);
  await assertRows([seedMarker, persistenceMarker]);
}

await assertStorage();
await assertEdgeFunction();
await assertServiceGraph();
console.log(`Supabase ${phase} behavior passed`);
