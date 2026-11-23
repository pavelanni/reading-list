# Newtypes and IP addresses

## A newtype makes the safe path the only path

Example: `WebClient` in `src/net.rs`.

Saved links are untrusted, so the client that fetches them must only reach
public addresses (SSRF protection). Wrapping `reqwest::Client` in
`struct WebClient { http, policy }` and taking `&WebClient` in
`fetch_article`, the shortener resolver, the bot and the worker means the
compiler rejects an unguarded client. A comment saying "use the guarded
client" can be forgotten; a type cannot.

## One check is not enough

- A custom DNS resolver filters what *hostnames* resolve to, and also
  catches DNS rebinding (a public name pointing at `192.168.1.1`).
- *IP-literal* URLs (`http://192.168.1.1/`) never reach the resolver, so
  `WebClient::request` and the redirect policy check them directly.
- A proxy resolves names itself, so the client sets `.no_proxy()`.

## IP classification traps

Example: `is_public`, `is_public_ipv4`, `is_public_ipv6`.

| Trap | What to do |
| ---- | ---------- |
| `IpAddr::is_global()` is unstable | Combine the stable `is_*` methods |
| CGNAT `100.64.0.0/10` has no stable method | Mask the second octet: `b & 0b1100_0000` |
| `::ffff:127.0.0.1` is IPv4 in IPv6 clothing | `to_ipv4_mapped()`, then apply the IPv4 rules |
| `to_ipv4()` also converts `::1` to `0.0.0.1` | Prefer `to_ipv4_mapped()` |
| NAT64 `64:ff9b::/96`, 6to4 `2002::/16` and `::a.b.c.d` also embed IPv4 | `embedded_ipv4`: extract it and apply the IPv4 rules |
| Prefix not a whole number of segments (`fec0::/10`) | Mask: `segments()[0] & 0xffc0 == 0xfec0` |

Splitting into `is_public_ipv4` and `is_public_ipv6` lets the IPv6 side
reuse the IPv4 rules for mapped addresses instead of duplicating them.
