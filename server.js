// ============================================================
//  blueblips-bridge — Node.js version
//  Spotify access point bridge for legacy iOS clients
// ============================================================

const express = require("express");
const cors    = require("cors");
const fetch   = require("node-fetch");
const net     = require("net");

const app  = express();
const PORT = process.env.PORT || 8080;
const HOST = (process.env.RENDER_EXTERNAL_URL || "localhost")
               .replace("https://", "")
               .replace("http://", "");

const SPOTIFY_USERNAME = process.env.SPOTIFY_USERNAME;
const SPOTIFY_PASSWORD = process.env.SPOTIFY_PASSWORD;
const CLIENT_ID        = process.env.SPOTIFY_CLIENT_ID     || "0a05255eb2224501aefeeceb5574c8a1";
const CLIENT_SECRET    = process.env.SPOTIFY_CLIENT_SECRET || "";

app.use(cors());
app.use(express.json());

// ── Token cache ───────────────────────────────────────────────
let cachedToken    = null;
let tokenExpiresAt = 0;

async function getSpotifyToken() {
  if (cachedToken && Date.now() < tokenExpiresAt) {
    return cachedToken;
  }

  const creds = Buffer.from(`${CLIENT_ID}:${CLIENT_SECRET}`).toString("base64");

  const res = await fetch("https://accounts.spotify.com/api/token", {
    method: "POST",
    headers: {
      "Authorization": `Basic ${creds}`,
      "Content-Type":  "application/x-www-form-urlencoded",
    },
    body: new URLSearchParams({
      grant_type: "client_credentials",
    }),
  });

  const data = await res.json();
  if (!data.access_token) throw new Error(data.error || "Token fetch failed");

  cachedToken    = data.access_token;
  tokenExpiresAt = Date.now() + (data.expires_in - 60) * 1000;
  return cachedToken;
}

// ── Route: GET / ──────────────────────────────────────────────
app.get("/", (req, res) => {
  res.json({
    status:  "ok",
    service: "blueblips-bridge",
    version: "0.1.0",
    host:    HOST,
  });
});

// ── Route: GET /status ────────────────────────────────────────
app.get("/status", async (req, res) => {
  try {
    await getSpotifyToken();
    res.json({ status: "ok", spotify: "reachable" });
  } catch (e) {
    res.json({ status: "error", message: e.message });
  }
});

// ── Route: GET /apresolve ─────────────────────────────────────
// The patched Spotify app calls this to find the AP server.
// We return our own address so it connects back to us.
app.get("/apresolve", (req, res) => {
  const apAddress = `${HOST}:4070`;
  console.log(`[apresolve] returning ${apAddress}`);
  // Spotify's apresolve returns plain text, one AP per line
  res.setHeader("Content-Type", "text/plain");
  res.send(apAddress);
});

// ── Route: GET /token ─────────────────────────────────────────
// Returns a fresh Spotify access token
app.get("/token", async (req, res) => {
  try {
    const token = await getSpotifyToken();
    res.json({ status: "ok", token });
  } catch (e) {
    res.status(500).json({ status: "error", message: e.message });
  }
});

// ── Route: GET /search ────────────────────────────────────────
// Proxy Spotify search so old iOS doesn't need TLS 1.2
app.get("/search", async (req, res) => {
  const { q, type = "track,artist,album", limit = 20 } = req.query;
  if (!q) return res.status(400).json({ error: "missing q param" });

  try {
    const token = await getSpotifyToken();
    const url   = `https://api.spotify.com/v1/search?q=${encodeURIComponent(q)}&type=${type}&limit=${limit}`;
    const r     = await fetch(url, {
      headers: { Authorization: `Bearer ${token}` }
    });
    const data = await r.json();
    res.json(data);
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── Route: GET /browse/featured ───────────────────────────────
app.get("/browse/featured", async (req, res) => {
  try {
    const token = await getSpotifyToken();
    const r     = await fetch("https://api.spotify.com/v1/browse/featured-playlists?limit=20", {
      headers: { Authorization: `Bearer ${token}` }
    });
    res.json(await r.json());
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── Route: GET /browse/new-releases ───────────────────────────
app.get("/browse/new-releases", async (req, res) => {
  try {
    const token = await getSpotifyToken();
    const r     = await fetch("https://api.spotify.com/v1/browse/new-releases?limit=20", {
      headers: { Authorization: `Bearer ${token}` }
    });
    res.json(await r.json());
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── Route: GET /artist/:id ────────────────────────────────────
app.get("/artist/:id", async (req, res) => {
  try {
    const token = await getSpotifyToken();
    const r     = await fetch(`https://api.spotify.com/v1/artists/${req.params.id}`, {
      headers: { Authorization: `Bearer ${token}` }
    });
    res.json(await r.json());
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── Route: GET /album/:id ─────────────────────────────────────
app.get("/album/:id", async (req, res) => {
  try {
    const token = await getSpotifyToken();
    const r     = await fetch(`https://api.spotify.com/v1/albums/${req.params.id}`, {
      headers: { Authorization: `Bearer ${token}` }
    });
    res.json(await r.json());
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ── Start HTTP server ─────────────────────────────────────────
app.listen(PORT, () => {
  console.log(`blueblips-bridge running on port ${PORT}`);
  console.log(`Host: ${HOST}`);
  console.log(`Client ID: ${CLIENT_ID}`);
});

// ── TCP Access Point Server (port 4070) ───────────────────────
// The old Spotify app connects here thinking it's a real AP.
// We handle the basic handshake and return a login response.
const apServer = net.createServer((socket) => {
  const addr = `${socket.remoteAddress}:${socket.remotePort}`;
  console.log(`[AP] connection from ${addr}`);

  socket.on("data", (data) => {
    console.log(`[AP] received ${data.length} bytes from ${addr}`);

    // The old app sends a ClientHello packet first
    // We respond with a minimal ServerHello + APWelcome
    // telling it login succeeded
    const welcome = Buffer.from([
      0x00, 0x00, 0x00, 0x04,  // length
      0xAC,                     // APWelcome message type
      0x00, 0x00, 0x00,        // padding
    ]);

    socket.write(welcome);
    console.log(`[AP] sent welcome to ${addr}`);
  });

  socket.on("error", (err) => {
    console.log(`[AP] socket error from ${addr}: ${err.message}`);
  });

  socket.on("close", () => {
    console.log(`[AP] disconnected: ${addr}`);
  });
});

apServer.listen(4070, () => {
  console.log("AP server listening on port 4070");
});

apServer.on("error", (err) => {
  console.error(`AP server error: ${err.message}`);
});
