import { connectRealtimeWebSocket } from "./realtime-websocket.mjs";
import { pathToFileURL } from "node:url";

export function realtimeReadinessTimeoutMilliseconds(rawRemainingSeconds) {
  const parsedRemainingSeconds = Number.parseInt(rawRemainingSeconds ?? "0", 10);
  if (!Number.isSafeInteger(parsedRemainingSeconds) || parsedRemainingSeconds <= 0) {
    throw new Error("SUPABASE_WAIT_REMAINING_SECONDS must be a positive integer");
  }
  return Math.max(1_000, Math.min(30_000, parsedRemainingSeconds * 1_000));
}

export async function verifyRealtimeReadiness({
  environment = process.env,
  connect = connectRealtimeWebSocket,
} = {}) {
  const anonKey = environment.BF_ANON_KEY;
  if (!anonKey) throw new Error("BF_ANON_KEY must be set");

  const base = environment.BF_SUPABASE_URL ?? "http://kong:8000";
  const timeoutMilliseconds = realtimeReadinessTimeoutMilliseconds(
    environment.SUPABASE_WAIT_REMAINING_SECONDS,
  );
  let websocket;
  try {
    websocket = await connect(
      `${base.replace(/^http/, "ws")}/realtime/v1/websocket?apikey=${encodeURIComponent(anonKey)}&vsn=1.0.0`,
      { timeoutMilliseconds },
    );
  } finally {
    websocket?.close();
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await verifyRealtimeReadiness();
}
