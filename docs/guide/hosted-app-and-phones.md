# The hosted app, phones and notifications

`silt open` shows the web UI of your own machine. Your server serves the same app at `/app/`, such as `https://silt.example.com/app/`, for any browser you sign in to, phones included. From there you can follow and answer the agents working on your machines, review merge requests, and continue a thread on one of your machines, all without being at it.

## What it shows
Sign in at the server's `/app/` as you sign in on its pages. It shows:
- the threads you can see, with their conversations;
- comments and merge requests;
- search, the inbox, orgs and projects.

Pinning, snoozing and settling there follow you to your machines.

What needs a checkout, such as diffs, a file's text and comments on code, is read through one of your machines that's online. Starting an agent and editing files happen on a machine, so the app asks one of yours to do them.

## Agents on your machines
Daemons tell the server how the agents they run stand. The hosted app, and the web UI on any of your machines, show which threads have an agent working or waiting on you elsewhere. Needs you lists them first. From there you can:
- answer a permission request, or approve a plan;
- prompt, steer, cancel or stop the agent;
- change its permissions or mode;
- sign the agent in when its own sign-in has expired: open the link the card shows, here or on your phone, and paste back the code ([signing an agent in](web-ui.md)). It needs "start and stop agents", since it decides which account the agent's work goes to.

The server passes each to the machine running the agent, which acts only on its own person's commands. An offline machine refuses them, except a prompt you choose to leave for when it's back.

A thread with no agent running can be continued on any of your online machines, with an agent that machine has: "Continue on" on the thread's page.

## Devices
Settings' Devices lists your signed-in devices and browsers:
- whether each is online, and which agents run on it;
- what it may do to your other devices: answer permissions, send prompts, start and stop agents, and review merge requests.

Rename a device or sign it out there too.

## A phone on your network, with no server
A phone on the same Wi-Fi as your machine can answer its agents and review without any server. Turn it on once, in Settings › Devices and sync under **Phones on this network**, or:

```sh
silt lan on
```

Your machine then listens for phones on port 7419, over HTTPS only, as `silt-<machine>-<4 letters>.local`, a name of its own that another machine of the same name doesn't share. If someone else's Silt on the same computer has 7419, yours takes the next free pair of ports and keeps it. Nothing else on the network can use it: only phones you allow get in.

1. **Trust your machine, once per phone.** Devices shows a QR code that downloads your machine's own certificate. Check the fingerprint the phone shows matches the one on Devices, then:
   - on an iPhone, install the downloaded profile in Settings, then turn it on in Settings › General › About › Certificate Trust Settings;
   - on Android, install it in Settings › Security › Encryption & credentials › Install a certificate › CA certificate.

   The certificate can only vouch for your machine's own `.local` name and private network addresses, never for a website or another device on the network. The phone's pages say whose Silt it is, such as *Silt on workstation (ada)*.

   Rather not install it? Tick **Pair without installing the certificate**. The phone then warns that the connection isn't private: go on past it, with **Show Details** then **visit this website** on an iPhone, or **Advanced** then **Proceed** on Android. The connection is still encrypted, but on that visit the phone can't tell your machine from something else on the network standing in for it; to check, compare the fingerprint in the warning's certificate details with the one under **Check it's really** on Devices. The phone works as it would with the certificate, except that it gets no notifications, can't add Silt to its Home Screen as an app, and may warn again after a while. Install the certificate later to get those back: the phone stays paired, and offers it when you open Silt.
2. **Pair the phone.** Choose **Next: pair the phone** and scan the new code. The phone waits until you choose **Allow** on your machine, in Devices or the prompt that shows on any page. A code works once, for 10 minutes. `silt lan pair` does the same in a terminal.

A paired phone may answer permissions and review, as a phone paired through a server may. Give it more, rename it or sign it out from its row in Devices, or `silt lan revoke <phone>`. Open Silt on the phone and turn notifications on: your machine pushes to it when an agent waits on a permission or a plan, or stops with an error. On an iPhone, add Silt to your Home Screen first.

If the phone can't open the page:
- check it's on the same network, not mobile data or a guest network;
- some networks don't pass `.local` names: use **Use the address instead**;
- a firewall may block port 7419, such as `ufw` (`sudo ufw allow 7419/tcp`). Under WSL 2, phones reach Windows rather than the distribution unless networking is mirrored.

`SILT_LAN_PORT` picks another port and `SILT_LAN_NAME` another machine name. **Start over** under the card signs every phone out and makes a new certificate and `.local` name. Several people on one network each pair with their own machine; to share threads between people, use a server. The design is in [ADR 0112](../adr/0112-phones-on-the-local-network.md).

## Phones through your server
The hosted app installs to a phone's home screen: open it in the phone's browser and add it from the browser's menu. On a phone it has a tab bar along the bottom, and the list of threads starts with what needs you, answerable in place. Merge requests show their files and diff, and take Seen ticks.

To sign a phone in without typing a password on it, pair it:
1. On a signed-in device, open Settings, then Devices, and choose "Pair a phone".
2. Scan the QR code with the phone, or open the address and type the short code. A code works once, for 10 minutes.

A paired phone may answer permissions and review. Give it more, or sign it out, from Settings' Devices.

## The inbox and notifications
The inbox gathers what you should hear about:
- merge requests, comments and `@mentions`;
- what your machines report:
  - an agent asking for a permission or waiting on a plan;
  - a turn finished;
  - an agent stopped with an error;
  - a machine gone offline with an agent at work;
  - a recovery thread made.

```sh
silt inbox
silt inbox read --all
```

Answer a permission, or approve a plan, from the inbox in place. On the inbox page, choose which kinds notify you; permissions, plans, errors and machines going offline do by default. Notifications come in two ways:
- **In an open tab.** An open Silt tab shows a browser notification while it isn't in front, once you allow it.
- **As a push.** The hosted app gets Web Push, even when it's closed, if the server can reach the browser's push service ([Self-hosting](../self-hosting.md) has the details).

The design is in [ADR 0026](../adr/0026-hosted-mode-and-remote-control.md) and [ADR 0028](../adr/0028-notifications-and-phones.md).
