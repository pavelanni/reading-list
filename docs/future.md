# Future ideas

Drafts and ideas, not commitments. Nothing here is planned or scheduled
yet. Feedback from friends and colleagues decides what moves forward.

## Reminders: bring saved links back

Saving a link is easy, and coming back to it is not. Today the service
only answers when asked, over MCP. Reminders would let the bot bring
unread stories back on a schedule.

### What a reminder looks like

```text
Reading list: 3 of 11 unread stories

[3] Secretive: SSH keys in the Mac Secure Enclave (idea, 12 days)
[2] TypeSafe AI launches System One Models (news, 5 days)
[1] SPIFFE and SPIRE: the cost of workload identity (news, 2 new)

[Read] [Drop] [Snooze 1w] under each story
```

- Each story shows its one-line summary and age, and whether new
  articles arrived since it was last marked read.
- Inline buttons (Telegram callback queries) mark a story read or
  dropped, or snooze it, without opening an agent. A tap reuses the
  same logic as `set_story_status` in `query.rs`.
- A footer line shows how many stories are left and links to
  `/unread` for the full list.

### Settings

| Setting | Example | Notes |
| ------- | ------- | ----- |
| Frequency | `daily`, `weekly`, `off` | Weekly takes a day: `weekly mon` |
| Time of day | `08:30` | Local time |
| Time zone | `Europe/Berlin` | An IANA name, so daylight saving time is handled |
| Max stories | `5` | Keeps the message short enough to act on |
| Filter | `kind=idea`, `tag=ai` | Optional; the same filters as `list_stories` |
| Quiet when empty | on | No message when nothing is unread |

Two places could hold the settings:

- **Environment variables** (`REMINDER_SCHEDULE=daily 08:30`) fit the
  current single-user setup and need no new table.
- **Bot commands** (`/remind daily 08:30`, `/remind max 5`,
  `/remind off`) stored in a `reminder_settings` table. They can be
  changed from the phone and are needed for household mode, where each
  member has a schedule.

Environment variables are enough for a first version. Commands can
follow.

### Which stories to show

Showing the newest stories every time would hide the old ones forever.
Some possible rules:

- **Rotate.** Record `last_reminded_at` per story and prefer stories
  that were shown least recently, so everything comes back eventually.
- **Mix ages.** Fill half the slots with the newest unread stories and
  half with the oldest ones.
- **Snooze.** A snoozed story is skipped until its snooze ends.
- **Nudge, then drop.** A story shown several times and never opened
  could get a "still interested?" prompt instead of piling up.

### Implementation notes

- A fourth service under the supervisor in `main.rs`, or a timer in the
  worker loop. It sleeps until the next due time and stops on the
  shared `CancellationToken`.
- Store `last_digest_at`, so a digest missed during a restart or
  upgrade is sent once at startup, not skipped and not sent twice.
- The selection rules are pure functions in `status.rs` or `query.rs`,
  unit-tested without Telegram or a clock (pass `now` in).
- Time zones need `chrono-tz` or a fixed UTC offset. A fixed offset is
  wrong for half the year wherever daylight saving time applies.
- `TelegramNotifier` can send a reminder today. Buttons need a callback
  handler in `bot.rs`.

## Household mode: more than one Telegram user

Two or more people who trust each other share one library, for example
a couple saving film recommendations for each other.

Allowing a list of user IDs is not enough on its own:

| Change | Why |
| ------ | --- |
| `TELEGRAM_ALLOWED_USER_IDS` as a list | Accept messages from every member |
| `added_by` column on `article` | Know who saved what; show and filter it over MCP |
| Notifications to the sender | `TelegramNotifier` sends to one fixed chat today |
| Reminder settings per member | Each member picks a schedule (see [Reminders](#reminders-bring-saved-links-back)) |

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

Reminders fit here too: "Friday evening: three films on your list".

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
- Cost report from `model_call` (per day, per model) over MCP.
