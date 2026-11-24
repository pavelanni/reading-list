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

The service runs as a rootless Podman container managed by a systemd
Quadlet. These steps are for RHEL 10; other systemd distributions with
Podman 5 work the same way.

### Build the image

```bash
git clone https://github.com/pavelanni/reading-list.git
cd reading-list
podman build -t reading-list .
```

### Create the secrets

Run these lines one at a time, not as a pasted block: each `read -rs`
waits for you to paste one value, and in a terminal without bracketed paste
it would take the next pasted command instead. The values, in order, are
the Telegram bot token, your numeric Telegram user ID, the OpenRouter API
key, and the [MCP token](#mcp-token). Nothing is echoed, and the values
never appear on a command line, so they stay out of your shell history.

```bash
read -rs BOT_TOKEN;  printf '%s' "$BOT_TOKEN"  | podman secret create reading-list-telegram-token -
read -rs USER_ID;    printf '%s' "$USER_ID"    | podman secret create reading-list-telegram-user -
read -rs OPENROUTER; printf '%s' "$OPENROUTER" | podman secret create reading-list-openrouter-key -
read -rs MCP_TOKEN;  printf '%s' "$MCP_TOKEN"  | podman secret create reading-list-mcp-token -
unset BOT_TOKEN USER_ID OPENROUTER MCP_TOKEN
```

`printf '%s'` stores the value without a trailing newline. A token that
ends in a newline will not match at authentication time.

### Install the Quadlet

```bash
mkdir -p ~/.config/containers/systemd
cp deploy/reading-list.container ~/.config/containers/systemd/
systemctl --user daemon-reload
systemctl --user start reading-list
loginctl enable-linger "$USER"   # keep it running after you log out
```

Check the service with `systemctl --user status reading-list` and the
logs with `journalctl --user-unit reading-list -f`. On RHEL the journal
is kept in memory by default (no `/var/log/journal`), so user services
log to the system journal and `journalctl --user -u` finds nothing.
Reading the system journal requires membership in `wheel`, `adm` or
`systemd-journal`. Alternatively, make the journal persistent with
`sudo mkdir -p /var/log/journal && sudo systemctl restart systemd-journald`;
journald then keeps a journal per user, and `journalctl --user -u` works too.

### Change the model or the unit

Edit the installed copy, `~/.config/containers/systemd/reading-list.container`
(for example `OPENROUTER_MODEL`, an ID from
[openrouter.ai/models](https://openrouter.ai/models)), then regenerate and
restart:

```bash
systemctl --user daemon-reload
systemctl --user restart reading-list
```

Open port 8787 in the firewall if clients connect from other machines:

```bash
sudo firewall-cmd --permanent --add-port=8787/tcp && sudo firewall-cmd --reload
```

The MCP endpoint is plain HTTP. Expose it only on your LAN: if the server
has other networks, publish the port on its LAN address only (see the
`PublishPort` comment in the unit).

### Upgrade

Pull, rebuild the image, then `systemctl --user restart reading-list`.
The database is in the `reading-list-data` volume and survives
rebuilds.

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
