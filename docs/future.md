# Future ideas

Drafts and ideas, not commitments. Nothing here is planned or scheduled
yet. Feedback from friends and colleagues decides what moves forward.

## Household mode: more than one Telegram user

Two or more people who trust each other share one library, for example
a couple saving film recommendations for each other.

Allowing a list of user IDs is not enough on its own:

| Change | Why |
| ------ | --- |
| `TELEGRAM_ALLOWED_USER_IDS` as a list | Accept messages from every member |
| `added_by` column on `article` | Know who saved what; show and filter it over MCP |
| Notifications to the sender | `TelegramNotifier` sends to one fixed chat today |

Everything else stays shared: one database, one MCP token, one
OpenRouter key.

## Domains beyond articles: a movie watchlist

The same pipeline, tuned for film recommendations:

- **Input without URLs.** "Watch *Perfect Days*, Wenders" has no link to
  fetch. A title-and-director extraction step, optionally followed by a
  lookup in a film database such as TMDB, would replace fetch and
  extract.
- **A different schema.** Title, year, director, country, genre and
  where to stream instead of extracted text. Stories become watchlist
  entries grouped by director, country or theme.
- **Different statuses.** `watched` and a rating instead of `read`.
- **Deployment.** One binary with a domain setting (prompts, extraction
  path, migrations), run as a separate instance with its own database,
  bot and Quadlet unit.

Open question: recommendations often arrive over WhatsApp. Forwarding
them to the Telegram bot is an extra step for the sender.

## A multi-user service

A larger change, worth it only if the feedback asks for it.

| Area | Today | Multi-user |
| ---- | ----- | ---------- |
| Tenancy | none | `user_id` on every row and every query; FTS results filtered per user |
| MCP auth | one shared bearer token | per-user OAuth, as in the MCP authorization spec |
| Model cost | one OpenRouter key | per-user quotas, or users bring their own key |
| Outbound fetches | SSRF guard, public addresses only | the same guard, plus rate limits and abuse controls |
| Storage | SQLite | SQLite at first; Postgres when the service scales out |
| Onboarding | env vars and a Quadlet | sign-up through the bot, a key per user |

## Smaller ideas

- Telegram commands for triage (`/unread`, `/read 12`). `query.rs` is
  already shared logic for this.
- A weekly digest message of unread stories.
- Cost report from `model_call` (per day, per model) over MCP.
