# The Blast protocol

A way for coding agents working in one repository to know about each other, with nothing running between them.

Version 1. The wire format below is the whole specification; an implementation that reads and writes these records interoperates with this one, whatever language it is in.

## The problem

Several agents now work in one repository at once: a few sessions of yours, a teammate's, a fleet of subagents inside one conversation. None of them can see the others. Two agents edit the same function and neither knows until the tests fail, or until a human notices.

The hosted answer to this is a server that every agent reports to. That is the right answer for a team across machines, and the wrong answer for one developer on one laptop who wants this to work in the next thirty seconds. This protocol is the local answer.

## The transport is the filesystem

Agents announce themselves by writing a small JSON file into a shared directory, and read the room by listing it. There is no daemon, no port, no broker, and no discovery step.

That is a deliberate choice, and these are the reasons:

- **Every harness can already do it.** Claude Code, Codex, Cursor and anything with a shell can write a file. Nothing needs installing or binding, and no permission dialog appears.
- **It survives crashes.** An agent killed mid-turn leaves a stale record, not a wedged socket. Records carry a timestamp and expire, so the directory heals with no supervisor.
- **One writer per file.** An agent only ever writes its own record, so no locking is needed. Concurrent writers cannot corrupt each other because they never touch the same path.
- **It spans machines unchanged.** Point the directory at a synced or mounted folder and the same protocol covers two laptops with no new code. Nothing in the format assumes one host.

The cost is honest: there is no push. A reader learns about a change when it next looks, which in practice is the agent's next hook event. For coordinating edits between agents that is soon enough, and it buys the absence of everything else.

## The directory

```
<repo>/.blast/peers/<agent>.json
```

`<agent>` is an opaque id, stable for as long as that agent lives. This implementation derives it as the first 12 hex characters of `sha256("<session>|<worker>")`, where `session` is the harness's session id and `worker` is the subagent id when the call came from inside one. Any scheme works as long as it is stable and collision-resistant, because a conversation running three subagents should appear as three hands, not one.

## The record

```json
{
  "v": 1,
  "agent": "58f138a2c901",
  "harness": "claude",
  "action": "editing",
  "path": "booking/pay.py",
  "symbols": ["charge"],
  "branch": "payments/minor-units",
  "pid": 41233,
  "ts": 1790284417.882
}
```

| field | meaning |
|---|---|
| `v` | format version, currently `1` |
| `agent` | the writer's stable id, matching the filename |
| `harness` | which agent runtime this is, for a human reading the room |
| `action` | `joined`, `reading`, `editing`, or anything an implementation finds useful |
| `path` | the repo-relative file the agent is working in, or `""` |
| `symbols` | the symbols being changed, when they are known |
| `branch` | the git branch, so two agents on different branches read as different work |
| `pid` | the writer's process id, for a human debugging a stuck record |
| `ts` | seconds since the Unix epoch, as a float |

Unknown fields must be ignored, not rejected. That is how this format grows without a version bump.

## Lifecycle

- **Announce.** Write the record when the session starts, and rewrite it whenever the action or the file changes.
- **Refresh.** Any write refreshes `ts`. An agent that goes quiet for longer than the stale window disappears on its own, which is the correct outcome.
- **Withdraw.** Delete the record at session end.
- **Expire.** A reader treats a record older than **180 seconds** as absent, and may delete it. Sweeping on read is what keeps the directory clean without anything scheduled.

## Writing safely

Never write a record in place. Write a temporary file in the same directory and rename it over the target; the rename is atomic, so a reader sees either the old record or the new one and never half of either. A reader that hits a malformed record should skip it rather than fail, because a peer may be running an implementation with a bug.

## What is not in the protocol

**No content.** Records name paths and symbols. They never carry file contents, diffs, or prompts. A peer record is safe to read.

**No authority.** This protocol reports; it does not arbitrate. Nothing here can stop an agent from writing, and no implementation should pretend otherwise. Deciding who yields is a policy question, and policy belongs above the transport.

**No history.** The directory is a snapshot of now. Anything durable, audited or cross-machine is a different problem, and a hosted coordinator is the honest answer to it.

## Coexisting with a coordinator

An implementation that finds a hosted coordinator configured for the repository should stand down rather than duplicate it. Briefing an agent twice about the same thing is worse than not briefing it at all: it doubles the tokens and teaches the agent to skim the channel.

This implementation checks for `.collide/config.json` and goes silent when it finds one, unless `BLAST_ALONE=1` is set. Other coordinators should be added to that check by the same rule: if something else already tells the agent what Blast would say, Blast says nothing.
