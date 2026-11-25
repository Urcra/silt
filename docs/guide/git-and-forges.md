# Git, pull requests and the Agent SDK

Silt links each commit to the turn that made it, but nothing outside Silt sees that by default. A project can turn on three things that show the link where people already look: notes on its commits, a comment on each thread's pull request, and, for apps built on the Claude Agent SDK, Silt as their session store. Commit messages stay as you and your agent wrote them.

## Notes on commits
With notes on, each commit a thread records gets a note in `refs/notes/silt`. The note names the thread, with a link to it, each turn that wrote lines of the commit, with its prompt, and who wrote which of the commit's lines.

Turn them on for a project from a clone of it:
```sh
silt project set --git-notes local   # write notes in this clone and the others
silt project set --git-notes push    # and push the notes to origin
silt project set --git-notes off
```

Changing it takes an owner of the project's org, as its other defaults do; Add a project in the web UI has the same choice. A project no server knows reads the clone's own setting instead: `git config silt.notes local`.

Notes stay in the clone until the project says `push`, since they show the thread's title and prompts to whoever can read the repository. Read them with git:
```sh
git log --notes=silt
git fetch origin refs/notes/silt:refs/notes/silt     # once, in another clone
git config --add remote.origin.fetch '+refs/notes/silt:refs/notes/silt'   # every fetch from then on
```

## Who wrote each line
`silt blame` shows each line of a file with the thread, turn, author and prompt that wrote it:
```sh
silt blame src/login.rs
silt blame --notes src/login.rs
```
```
   1 0eacd06e                                                       | fn main() {}
   2 0f3c1a2b9d4e t1  claude-code  Fix the login redirect when the  | fn redirect() {}
   3 0f3c1a2b9d4e t2  person       Now add a test                   | fn by_hand() {}
```

Inside a checkout of a thread, it reads the thread's own history, uncommitted lines included. Anywhere else, and for lines from before the thread, it runs `git blame` and reads each commit's note. That works in any clone that fetched the notes, with no daemon and no server. `--notes` reads only the notes. A line that shows only a commit came from git, with nothing in Silt about it.

## Comments on pull requests
A project can ask its server to comment on its threads' pull requests on GitHub, GitLab or Forgejo:
```sh
silt project set --forge-comments on
```

The server then keeps one comment on the open pull request from each thread's branch. It names the thread with a link to it, its turns and commits, the summary of its open merge request, and its forks and merges. It edits that comment as the thread changes rather than posting again. A private thread is never commented on, and private forks are left out.

The server needs a token for each forge; its admin sets them up as [Self-hosting](../self-hosting.md) describes.

## Silt as the Agent SDK's session store
An app built on the Claude Agent SDK keeps its sessions on the host that ran them. With Silt as its session store, a session started on one host resumes on another, and it shows up among your threads, searchable and readable like one you recorded.

1. Copy the adapter for your language from [`packaging/agent-sdk`](https://github.com/Urcra/silt/tree/main/packaging/agent-sdk) into your app.
2. Give it your server's URL and a device token. `silt auth login` on the app's host writes one to `servers.json`, or make one with `silt server pair`.
3. Pass it as the session store:
   ```ts
   const store = new SiltSessionStore({ server: "https://silt.example", token: process.env.SILT_TOKEN! });
   for await (const message of query({ prompt, options: { sessionStore: store } })) { /* … */ }
   ```

Each session becomes a private thread of the token's person, with its subagent transcripts beside it. Entries are redacted before they're stored, so an entry holding a secret comes back with the secret masked. `silt resume` can't take such a thread yet, since it has no checkout to resume in.
