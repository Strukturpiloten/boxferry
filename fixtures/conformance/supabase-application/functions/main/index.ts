const encoder = new TextEncoder();

Deno.serve({ port: 9000 }, async (request: Request) => {
  const digest = await crypto.subtle.digest("SHA-256", encoder.encode(await request.text()));
  const checksum = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  return Response.json({ application: "boxferry-supabase", checksum });
});
