# Deployment

The service runs as a rootless Podman container managed by a systemd
Quadlet. These steps are for RHEL 10; other systemd distributions with
Podman 5 work the same way.

## Get the image

Releases publish `ghcr.io/pavelanni/reading-list` for `linux/amd64` and
`linux/arm64`, tagged by version (`0.2.0`, `0.2`) and `latest`. Each
release's notes show how to verify its signature and provenance.

```bash
git clone https://github.com/pavelanni/reading-list.git   # for the unit file
cd reading-list
podman pull ghcr.io/pavelanni/reading-list:latest
```

To build from source instead, run `podman build -t reading-list .` and
set `Image=localhost/reading-list:latest` in the unit.

## Raspberry Pi

The `arm64` image runs on a Raspberry Pi 3, 4 or 5 with a 64-bit OS,
such as Raspberry Pi OS (64-bit) or Fedora. Install Podman 5 and follow
the same steps. 32-bit systems are not supported.

## Create the secrets

Run these lines one at a time: each `read -rs` waits for one pasted
value (without bracketed paste it would swallow the next command). The
values are the Telegram bot token, your numeric Telegram user ID, the
OpenRouter API key, and the [MCP token](../README.md#mcp-token). Nothing is echoed
or kept in shell history.

```bash
read -rs BOT_TOKEN;  printf '%s' "$BOT_TOKEN"  | podman secret create reading-list-telegram-token -
read -rs USER_ID;    printf '%s' "$USER_ID"    | podman secret create reading-list-telegram-user -
read -rs OPENROUTER; printf '%s' "$OPENROUTER" | podman secret create reading-list-openrouter-key -
read -rs MCP_TOKEN;  printf '%s' "$MCP_TOKEN"  | podman secret create reading-list-mcp-token -
unset BOT_TOKEN USER_ID OPENROUTER MCP_TOKEN
```

`printf '%s'` stores the value without a trailing newline. A token that
ends in a newline will not match at authentication time.

## Install the Quadlet

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

## Change the model or the unit

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
`PublishPort` comment in the unit). To encrypt it, see the next section.

## Encrypt the connection

The bearer token and your reading data cross the network in clear text.
Choose by where your clients are:

| Clients | Option |
| ------- | ------ |
| Trusted home LAN | Plain HTTP. Nothing to set up. |
| Away from home | A VPN such as WireGuard or Tailscale. Plain HTTP inside the tunnel is already encrypted. |
| Shared or untrusted network | A TLS proxy in front of the server, below. |

### TLS with Caddy

[Caddy](https://caddyserver.com) can terminate TLS on the same host and
forward `/mcp` to the container. `deploy/Caddyfile.example` is the starting
point. Install `caddy`, then:

1. Edit the host name in the Caddyfile and install it as
   `/etc/caddy/Caddyfile`.
2. Open ports 80 and 443 in the firewall. Caddy uses port 80 for the
   certificate challenge.
3. Start it with `sudo systemctl enable --now caddy` and watch
   `journalctl -u caddy` for "certificate obtained successfully".
4. Publish the app on loopback only (`PublishPort=127.0.0.1:8787:8787`),
   restart the unit and close port 8787 in the firewall.

Let's Encrypt only issues certificates for public names. For a host that
the Internet cannot reach, use the DNS challenge (a Caddy build with your DNS
provider's module) or a private CA. The commented block in the Caddyfile
shows an ACME private CA such as
[step-ca](https://smallstep.com/docs/step-ca/). With a private CA, copy its
root certificate to the Caddy host, and point `NODE_EXTRA_CA_CERTS` at the
root certificate on each client that runs `mcp-remote`: Node ignores the
operating system trust store.

If another service already owns port 443, set `https_port` in the global
options and add the port to the site address. SELinux then blocks HTTP/3 on
that port, so also set `protocols h1 h2`, as the example comments show.

## Upgrade

The database lives in the `reading-list-data` volume and survives
upgrades. Schema migrations run at startup and only go forward, so back
up the volume first if you might roll back. Stop the service before the
export: SQLite runs in WAL mode, and a copy taken while it writes can be
inconsistent.

```bash
podman pull ghcr.io/pavelanni/reading-list:latest   # the tag in the unit
systemctl --user stop reading-list
podman volume export reading-list-data -o backup.tar
systemctl --user start reading-list
```

For an image built from source, rebuild it instead of pulling.
