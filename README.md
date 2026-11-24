# Reading list

A personal reading inbox. Share a link to a Telegram bot; the service
fetches the article, groups it into a *story* with related articles,
and writes a summary with an OpenRouter model. Coding agents such as
Claude Code or OpenCode read and triage the stories over MCP.

One binary runs three services: the Telegram bot (long polling), the
processing worker, and the MCP server on port 8787. Data lives in one
SQLite file.

## Saving links

| You send | Result |
| -------- | ------ |
| A link | Saved; the bot replies `Saved` |
| A link with `#tag` words | Saved with those tags: `Saved, tagged #tag` |
| A line starting with `>>` | That line becomes your note on the article |
| A reply to a bot's `Saved` message | The reply text is added as a note: `Note added` |
| The same link again | Tags and note are merged into the existing article |

Text other than tags and `>>` lines is kept as share text, separate
from your note. The bot ignores messages from anyone except you.

## Setup

### Telegram bot

1. In Telegram, message [@BotFather](https://t.me/BotFather), send
   `/newbot`, and follow the prompts. Keep the token it gives you.
2. Find your numeric user ID: message [@userinfobot](https://t.me/userinfobot)
   and copy the `Id` it returns.
3. Open your new bot and send `/start`, so it is allowed to message you.

### OpenRouter

Create an API key at [openrouter.ai/keys](https://openrouter.ai/keys)
and set a credit limit on it. When the key runs out of credit, the bot
tells you once that summaries are paused; saving keeps working and the
queue resumes when credit returns.

### MCP token

Generate a long random token (at least 32 characters):

```bash
openssl rand -hex 32
```

## Deployment

A rootless Podman container under a systemd Quadlet, from the
multi-arch release image (`linux/amd64`, `linux/arm64`, including
Raspberry Pi with a 64-bit OS) or built from source. See
[docs/deployment.md](docs/deployment.md).

## Configuration

| Variable | Required | Default |
| -------- | -------- | ------- |
| `TELEGRAM_BOT_TOKEN` | yes | |
| `TELEGRAM_ALLOWED_USER_ID` | yes | |
| `OPENROUTER_API_KEY` | yes | |
| `OPENROUTER_MODEL` | yes | |
| `MCP_BEARER_TOKEN` | yes | |
| `OPENROUTER_BASE_URL` | no | `https://openrouter.ai/api/v1` |
| `MCP_LISTEN_ADDR` | no | `0.0.0.0:8787` |
| `DATABASE_PATH` | no | `/data/reading-list.db` |
| `RUST_LOG` | no | `info` |

## Connecting agents

The MCP server is at `http://<server>:8787/mcp` and requires the header
`Authorization: Bearer <MCP_TOKEN>`.

On the machine that runs the agent, set `READING_LIST_TOKEN` to the same
value you stored in the `reading-list-mcp-token` secret:

```bash
read -rs READING_LIST_TOKEN; export READING_LIST_TOKEN
```

### Claude Code

The shell expands the variable when you run this command, so Claude Code
saves the token itself in `~/.claude.json`. After that the variable is no
longer needed.

```bash
claude mcp add --transport http reading-list http://SERVER:8787/mcp \
  --header "Authorization: Bearer $READING_LIST_TOKEN"
```

### Claude Desktop

Desktop cannot send a custom header to a remote server, so run the
`mcp-remote` bridge on your machine. In `claude_desktop_config.json`:

```json
"mcpServers": {
  "reading-list": {
    "command": "npx",
    "args": ["-y", "mcp-remote", "https://SERVER/mcp",
             "--header", "Authorization:${AUTH_HEADER}"],
    "env": { "AUTH_HEADER": "Bearer <MCP_TOKEN>" }
  }
}
```

Use an `https://` URL (see [TLS with Caddy](docs/deployment.md)). A plain
`http://` URL also needs `--allow-http`. To keep the token out of the file,
start the bridge from a small shell wrapper that reads it from your
password manager or the macOS Keychain. Desktop runs with a minimal
`PATH`, so give the full path to `npx`.

### OpenCode

OpenCode reads `READING_LIST_TOKEN` from its environment at every start,
so export it from your shell profile or a secrets manager. Add this to
`opencode.json`:

```json
{
  "mcp": {
    "reading-list": {
      "type": "remote",
      "url": "http://SERVER:8787/mcp",
      "headers": { "Authorization": "Bearer {env:READING_LIST_TOKEN}" }
    }
  }
}
```

### Tools

| Tool | Purpose |
| ---- | ------- |
| `list_stories` | Stories by status, age, tag or kind (default: unread, last 7 days) |
| `get_story` | A story's summary and its articles |
| `get_article` | One article with its text and your note |
| `search` | Keyword search over stories and articles |
| `set_story_status` | Mark a story unread, read or dropped |
| `list_failed` | Articles that could not be fetched or summarized |
| `reprocess_article` | Queue an article again with a fresh retry budget |

A read story returns to unread when a new article adds information;
a dropped story stays dropped.

## Development

See `CLAUDE.md` for the toolchain, commands and crate versions.

## License

Apache License 2.0. See `LICENSE`.
