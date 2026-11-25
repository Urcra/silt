# 0112: Phones on the local network

- Status: accepted
- Date: 2026-11-21
- Amends: [ADR 0011](0011-history-tools-search-and-web-ui.md) (the web UI only on 127.0.0.1) and [ADR 0028](0028-notifications-and-phones.md) (phones only through the hosted app).
- Amended by: [ADR 0113](0113-pairing-a-phone-without-its-certificate.md): Pairing a phone without its certificate

## Context
A phone reaches Silt only through a server's hosted app (ADR 0026, ADR 0028). Someone who works on one machine, with no server or with Local only workspaces (ADR 0065), has no way to answer an agent from the sofa. Setting up a server just for that is too much.

The phone and the machine are usually on the same network, but the daemon's UI listens only on `127.0.0.1`, behind a token handed out per start. Phone browsers treat plain HTTP to a LAN address as insecure. They won't install it as a PWA, register a service worker, take a push subscription or open the microphone there. A self-signed certificate behind a warning gets none of those either.

## Decision
**Off by default, turned on per machine.** Settings › Devices has "Phones on this network", and `silt lan on|off|status` does the same. While it's on, the daemon opens a second listener on every interface, at a fixed port, so a phone's saved app keeps its address across restarts. It's 7419 unless that or the next is taken, as by another person's Silt on the same computer: then the first free pair above, two at a time. The port is remembered in `lan/config.json` once it's used, and never moved after, as phones know it; `SILT_LAN_PORT` fixes it. The loopback listener and its token stay as they are.

**HTTPS only, from a CA of the machine's own.**
- The first time it's turned on, the daemon makes a CA (ECDSA P-256, `rcgen`) in `$SILT_HOME/lan/`, its key readable only by the person. The CA is name-constrained to this machine's own `.local` name and the private address ranges (10/8, 172.16/12, 192.168/16, fc00::/7, fe80::/10), which addresses change within. So if its key leaks, or a phone trusts another person's machine, it can't vouch for any site on the internet nor for anything else on the network, such as a printer or a NAS.
- The CA signs a leaf certificate for the machine's `.local` name and its current private addresses. The leaf lasts 90 days. It's made again when it's due or when the addresses change, while the CA, and so the phone's trust in it, stays.
- The listener speaks TLS only (`rustls`). A second plain-HTTP port, `SILT_LAN_PORT + 1`, serves nothing but the CA certificate, an iOS profile carrying it, and a page with install steps, so a phone can fetch the CA before it trusts it. The CA's SHA-256 fingerprint shows on that page and on the Devices page, for the person to compare.

**Found by name, with addresses as the fallback.** The daemon publishes `silt-<hostname>-<suffix>.local` and a `_silt._tcp` service over mDNS (`mdns-sd`). It uses a name of its own so it doesn't clash with avahi, Bonjour or Windows answering for the host's name. The suffix, four hex digits made once and remembered with the port, keeps two machines of the same host name apart, and two people's Silt on one computer. The name stays as long as the authority, which is constrained to it; Start over makes a new one of each. The pairing QR uses the name. "Use the address instead" shows a QR for the current IP, for a phone or network that doesn't resolve `.local`. A phone's saved app is tied to the address it was paired on, and the page says so. `SILT_LAN_NAME` gives the machine another name than its host name, for phones and in the `.local` name.

**Pairing.**
1. On the machine, Devices shows the CA step, then a QR code for `https://silt-<host>.local:7419/pair#code=…`. A code works once, for 10 minutes.
2. The phone opens it, and the machine's open UI shows "A phone asks to pair", with the phone's browser and system, and Allow or Deny. A code alone pairs nothing, so a QR someone glimpses is no use. The phone's pages, the profile it installs and the mDNS service name whose Silt it is, by the OS user, as `Silt on workstation (ada)`, so people sharing a network or a computer pick their own.
3. The phone's page waits, looking again every two seconds, tied to its ask by a cookie. Once allowed, the phone gets a session cookie of its own (`silt_lan`, HttpOnly, Secure, SameSite=Lax, as the server's is: Strict would drop it when a phone's camera app opens the link). The daemon keeps only the token's hash, with the phone's name and capabilities, in `$SILT_HOME/lan/phones.json`. Requests that change something must carry `X-Silt-Request: 1` and a matching `Origin`, as on the server (ADR 0026). `silt lan pair` shows the code in a terminal and asks there.

**The same capabilities as a server-paired phone.** A LAN-paired phone may answer permissions and review by default (ADR 0028), and the person gives it more from its card. The LAN listener checks every request against the phone's capabilities before it reaches the daemon (`Request::needs`, in `lan/needs.rs`, which names every request, so a new one has to be placed). Reading the person's threads needs nothing more; changing the machine itself, its settings, projects, plugins and what it shares, no phone may. A request on the LAN listener with no phone behind it is refused, so work a socket moves to a task of its own carries its phone along. The LAN listener also checks the `Host` against the machine's `.local` name and its addresses.

**Web Push from the daemon.** The daemon keeps a VAPID key of its own, beside the CA, and each phone's push subscription with its pairing. When an agent here starts waiting on a permission or a plan, or stops with an error, it pushes to the paired phones, encrypted as RFC 8291 says. These are the kinds ADR 0028 notifies by default, with the same deduplication. The push goes through the phone browser's push service, so the machine needs outbound HTTPS to it; without that, the phone still gets notifications while Silt is open on it. A daemon signed in to a server leaves pushes to phones paired through the server to the server, as now.

**What the phone sees.** The daemon's own app, at `/` on the LAN origin, in its phone layouts, with the manifest and service worker the hosted app has. Threads running on the person's other machines are reached as from the machine's own UI: through its server, if it has one (ADR 0026, "Local daemon first").

## Consequences
- New state in `$SILT_HOME/lan/`: the CA and its key, the leaf, the VAPID key, and `phones.json`. Signing a phone out removes its entry, and its cookie stops working at once. "Forget this network's phones" deletes the CA too, and every phone needs the new one.
- Installing the CA is the one fiddly step: a profile and a trust toggle on iOS, and "CA certificate" in Android's security settings. The guide walks through both. After that, the phone gets everything the hosted app has.
- People sharing a network each pair their phones with their own machine: a phone gets in only once that machine's person allows it, and a paired phone acts as that person, with the capabilities they gave it. Several people on one machine, each with their own access, is the server's job, with its accounts.
- Firewalls may block port 7419. The Devices page can't tell from the inside, so it names the likely culprit (macOS's prompt, `ufw`/`firewalld`, Windows Defender). WSL 2's NAT keeps the LAN out unless mirrored networking is on.
- A web page on the same network can't ride the phone's session: the cookie is SameSite=Strict and the CSRF header is required. Another device on the network sees only TLS.
- Agents in Silt's sandbox still can't reach the LAN (ADR 0093), this listener included.
- The daemon builds its pushes with the same `web-push-native` steps as the server, in a module of its own (`lan/push.rs`): sharing the server's would have meant a crate for some forty lines.
