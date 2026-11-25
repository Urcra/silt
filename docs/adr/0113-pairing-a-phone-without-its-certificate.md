# 0113: Pairing a phone without its certificate

- Status: accepted
- Date: 2026-11-21
- Amends: [ADR 0112](0112-phones-on-the-local-network.md) (each phone trusts the machine's CA before it pairs).

## Context
ADR 0112 has each phone install the machine's own CA before it pairs: a profile and a trust toggle on iOS, a buried setting on Android. It's the one fiddly step, and many people stop there.

We looked at getting each machine a public name and a Let's Encrypt certificate from a shared service instead. It would need a domain, a DNS provider holding a token for it, and trust in whoever runs the service, since they could get a certificate for any machine's name. It would also fail where routers drop public names that point at private addresses. One certificate shared by every machine was ruled out too: its key would be public, letting anyone stand in for any machine, and the CA would revoke it.

Plain HTTP was ruled out as well. The phone's session cookie and every answer it gives would cross the network in the clear. Anyone else on a shared Wi-Fi could take the session and approve an agent's permissions, which means running commands on the machine.

## Decision
**A checkbox on the certificate step: "Pair without installing the certificate".** Checked, Devices goes straight to the pairing QR. The phone opens the same HTTPS listener, shows its browser's warning for a certificate it doesn't trust, and the person goes on past it. The listener, its certificate, the cookie and every check stay as ADR 0112 has them. The choice is remembered on the machine's browser, as the step the person last took.

**The fingerprint is still offered.** The pairing step shows the certificate's fingerprint and where each browser shows it behind its warning, for whoever wants to compare. The step says what going on past the warning means: on that visit the phone can't tell the machine from something else on the network standing in for it.

**What the phone gives up, said where the person chooses.** A browser keeps a page behind a certificate warning from registering a service worker, so the phone gets no notifications and can't install Silt as an app. Some browsers ask again after a while. The checkbox's note says so, and that installing the certificate later brings all of it back without pairing again.

**The phone says what's missing.** On a paired phone whose service worker can't register, the notifications banner offers the certificate step (the plain-HTTP `/trust` page) rather than Add to Home Screen. Devices' row for a phone without notifications already says so.

## Consequences
- No new state, endpoint or protocol change: the daemon can't tell a phone that trusts the CA from one that went past the warning, and doesn't need to.
- On the visit where the person goes past the warning, someone actively intercepting on the network could stand in for the machine and see the pairing code. They still need the person's Allow on the machine to pair, and the Allow card shows the phone's browser and system. A passive listener sees only TLS.
- What browsers do behind an accepted warning needs checking on real phones: whether iOS Safari and Android Chrome keep the `Secure` cookie, how long each remembers the exception, and whether the microphone works there.
- The guide gets a short "without the certificate" path beside the existing one.
