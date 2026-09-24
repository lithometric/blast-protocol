# Blast

**Your coding agent finds out what it broke, in files it never opened, before you read a word of its summary. It costs zero model tokens, because no model is involved in finding out.**

Blast is a local protocol and a single binary. Agents working in one repository announce themselves to each other through a shared directory, and every edit is checked against the call graph of everything that depends on it. No server, no account, no sign-up, nothing to host.

```
$ blast radius booking/pay.py::charge
booking/pay.py::charge: 3 dependent(s)
  hop 1  create_booking               booking/api.py
  hop 1  test_charge                  tests/test_pay.py
  hop 2  test_create_booking          tests/test_api.py
covered by: tests/test_pay.py, tests/test_api.py
```

Change that function's signature and the next thing your agent reads is this, unprompted:

```
Blast: your change to booking/pay.py breaks 2 tests in files you did not edit:
tests/test_pay.py::test_charge, tests/test_api.py::test_create_booking.
Fix them or tell the user what you are leaving broken, before you move on.
```

`tests/test_api.py` is two hops away, through a file the agent never opened. That is the failure Blast exists for: agents are confident, and the damage lands somewhere they were not looking.

## Install

```sh
curl -sSfL https://raw.githubusercontent.com/lithometric/blast-protocol/main/install.sh | sh
cd your-repo && blast init
```

`blast init` wires the hook into `.claude/settings.json` and indexes the tree. That is the whole setup. Cursor and Codex are `blast init --harness cursor` and `--harness codex`.

## What it does

- **Tells the agent what it broke.** On every write, Blast walks the call graph backwards from the symbols that changed, finds the tests covering them, and runs those tests in a detached process. The turn never waits. Whatever the tests said arrives on the agent's next event.
- **Tells agents about each other.** Every agent in the repository announces what it is doing to `.blast/peers/`. A second agent starting work is told who else is here and on what file, so two agents stop editing the same function in silence.
- **Costs no model tokens to produce.** Nothing here asks a model anything. The graph comes from a parser, the verdict comes from your test runner, and the only tokens spent are the handful of words injected. Compare that to an agent discovering the same fact by grepping for callers and reading the files it finds.

## Commands

```
blast init      wire the hook into this repo's harness settings, and index
blast index     rebuild the symbol and call graph
blast radius    what calls this, and which tests cover it
blast peers     who else is working in this repository right now
blast check     run the tests covering these files, by hand
blast uninstall remove our hook entries and nothing else
```

## What it stores

Structure only. Source files are read, parsed in memory, and dropped. `.blast/index.json` holds symbol names, signatures, line spans, docstrings and call edges. It never holds file contents, so the index of a private repository is not a copy of it.

Ten languages: Python, TypeScript, TSX, JavaScript, Go, Rust, Java, C#, Ruby, C, C++, PHP. Every grammar is compiled into the binary, so there is no runtime to install.

## Alongside a hosted coordinator

Blast is the local half of a problem that also has a hosted answer, and one of those answers is a superset of the other. If a coordinator is configured in the repository, Blast detects it and stands down: it says nothing, runs nothing, and costs nothing, so you are never briefed twice or billed for both. `BLAST_ALONE=1` overrides that if you want the local check anyway.

It also never fights over the settings file. Blast adds and removes only entries whose command is its own, and leaves every other tool's hooks exactly as it found them.

## Configure

`.blast/config.json`:

```json
{ "test": "python -m pytest -q" }
```

Blast guesses the test command from what is on disk. When it guesses wrong, this is the fix.

## License

MIT. See [PROTOCOL.md](PROTOCOL.md) for the wire format, which is deliberately simple enough to reimplement in an afternoon.
