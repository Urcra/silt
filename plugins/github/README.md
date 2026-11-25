# GitHub

Lets an agent in Silt's sandbox work with GitHub: clone and fetch, push, and read and change issues and pull requests. It never holds a token: Silt adds yours to each request it allows, and keeps the git work on mirrors of its own.

## Setting it up
1. Keep a token as a credential of the workspace, such as `gh auth token`'s:
   ```bash
   silt credential add work-github --workspace work --command "gh auth token"
   ```
2. Name the repos each environment covers, and give it the credential:
   ```bash
   silt plugin environment silt.github acme "acme/*"
   silt plugin credential silt.github token work-github --environment acme
   ```
3. Turn it on: `silt plugin enable silt.github --workspace work`.

## What the agent does
- `github_clone` mirrors a repo for the thread at `/silt/github/<owner>/<repo>.git`, read-only. The agent clones it into its work area, `/silt/work`, and works there; `github_fetch` brings the mirror up to date.
- `github_push` pushes a branch of the agent's clone. Silt fetches the agent's commit into the mirror with git's `upload-pack` alone, the only git command run in a repo the agent can write, and pushes exactly that commit from the mirror, only if the branch is still where it was.
- The rest are calls to GitHub's API: reading repos, files, issues, pull requests, checks and search; and opening or editing issues, commenting, opening, editing, reviewing and merging pull requests, running workflows and making releases.

## What decides
Each call is an action, and each action is allowed, asked or never by your rules, per environment and by repo or branch:

| Action | Starts as |
|---|---|
| reads: clone, fetch, repo, file, issues, pulls, checks, search | allowed |
| push (a branch that isn't the default or protected, moving forward) | asked |
| push to the default or a protected branch | asked, each time |
| force-push | never |
| delete a branch, issues, comments, pull requests, reviews, workflows | asked |
| merge | asked, each time, bound to the head shown |
| release | never |

`silt plugin rule silt.github push allow --where branch='ada/*'` lets pushes to your own branches go without asking.

## What you see
A push's card shows the repo, the branch moving from one commit to the next, its commits and the files they change. An API change's card shows exactly what's sent, with hidden characters made visible.

The sandbox holds no GitHub token: `GH_TOKEN`, `GITHUB_TOKEN` and the like never pass in, and where this plugin is on, Silt's gateway adds no credential of yours for GitHub's hosts. Anonymous reads of public repos still work.
