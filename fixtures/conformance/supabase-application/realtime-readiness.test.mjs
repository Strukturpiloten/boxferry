import assert from "node:assert/strict";
import test from "node:test";

import {
  realtimeReadinessTimeoutMilliseconds,
  verifyRealtimeReadiness,
} from "./realtime-readiness.mjs";

test("readiness uses the Kong route, remaining deadline, and closes its socket", async () => {
  const calls = [];
  const socket = {
    closed: false,
    close() {
      this.closed = true;
    },
  };
  await verifyRealtimeReadiness({
    environment: {
      BF_ANON_KEY: "public-test-key with punctuation/+",
      BF_SUPABASE_URL: "http://kong:8000",
      SUPABASE_WAIT_REMAINING_SECONDS: "17",
    },
    connect: async (...arguments_) => {
      calls.push(arguments_);
      return socket;
    },
  });

  assert.deepEqual(calls, [
    [
      "ws://kong:8000/realtime/v1/websocket?apikey=public-test-key%20with%20punctuation%2F%2B&vsn=1.0.0",
      { timeoutMilliseconds: 17_000 },
    ],
  ]);
  assert.equal(socket.closed, true);
});

test("readiness caps a connection attempt at thirty seconds", () => {
  assert.equal(realtimeReadinessTimeoutMilliseconds("90"), 30_000);
  assert.equal(realtimeReadinessTimeoutMilliseconds("1"), 1_000);
});

test("readiness rejects missing or invalid bounded inputs", async () => {
  await assert.rejects(
    verifyRealtimeReadiness({
      environment: { SUPABASE_WAIT_REMAINING_SECONDS: "17" },
      connect: async () => assert.fail("connection must not be attempted without a key"),
    }),
    /BF_ANON_KEY must be set/,
  );
  for (const remaining of [undefined, "0", "invalid"]) {
    await assert.rejects(
      verifyRealtimeReadiness({
        environment: { BF_ANON_KEY: "public-test-key", SUPABASE_WAIT_REMAINING_SECONDS: remaining },
        connect: async () => assert.fail("connection must not be attempted outside its deadline"),
      }),
      /SUPABASE_WAIT_REMAINING_SECONDS must be a positive integer/,
    );
  }
});

test("readiness propagates a connection failure without retrying or mutating", async () => {
  const failure = new Error("bounded generic connection failure");
  let attempts = 0;
  await assert.rejects(
    verifyRealtimeReadiness({
      environment: {
        BF_ANON_KEY: "public-test-key",
        SUPABASE_WAIT_REMAINING_SECONDS: "30",
      },
      connect: async () => {
        attempts += 1;
        throw failure;
      },
    }),
    (error) => error === failure,
  );
  assert.equal(attempts, 1);
});
