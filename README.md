# blueblips-bridge

A librespot bridge server that lets legacy Spotify iOS clients (iOS 5/6) authenticate and connect through a modern proxy.

## How it works

```
Old iPhone (Spotify 0.6.x)
    ↓  hits apresolve
x.blueblips.com  (Cloudflare Worker)
    ↓  returns this server's address
blueblips-bridge  (this server on Render)
    ↓  speaks libspotify binary protocol
Spotify servers  (modern auth via librespot)
```

## Deploy to Render (free)

1. Fork this repo on GitHub
2. Go to render.com → New → Web Service
3. Connect your GitHub and select this repo
4. Add these environment variables:
   - `SPOTIFY_USERNAME` — your Spotify email
   - `SPOTIFY_PASSWORD` — your Spotify password
5. Click Deploy

## Environment Variables

| Variable | Description |
|----------|-------------|
| `SPOTIFY_USERNAME` | Your Spotify account email |
| `SPOTIFY_PASSWORD` | Your Spotify account password |
| `PORT` | HTTP port (Render sets this automatically) |

## Endpoints

| Route | Description |
|-------|-------------|
| `GET /` | Status page |
| `GET /apresolve` | Returns AP server address for old clients |
| `GET /status` | Health check JSON |
| `GET /token` | Get a fresh Spotify access token |

## After deploying

Update your Cloudflare Worker (`x.blueblips.com`) config:
```js
BRIDGE_URL: "https://your-app.onrender.com"
```

Then update the patched IPA's apresolve to point to:
```
your-app.onrender.com
```
