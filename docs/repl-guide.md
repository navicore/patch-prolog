# REPL Guide

`plgr` is an interactive Prolog loop — define clauses, run queries, walk
solutions — built on the same compiler as everything else. It does **not**
interpret: every program change is compiled to a fresh native binary, and
queries run against that binary. What you test in the REPL is exactly what
`plgc build` would ship.

Because it drives the compiler, `plgr` needs `plgc` on your `PATH` (or set
`$PLGC` to its location). Start it with no arguments:

```sh
plgr
```

```
plgr — patch-prolog REPL.  :help for commands, :quit to exit.
plg> ▮
```

## Two kinds of input

The prompt is `plg> ` — deliberately neutral, because it accepts two
different things:

- **A clause, rule, or directive** — anything ending in `.` that isn't a
  query. This *edits your program* and recompiles.
- **A query** — `?- Goal.` This *runs* against the current program and
  shows solutions. It does not change anything.

You type the `?-` yourself; that's what tells the REPL "run this" rather
than "remember this."

```
plg> depends_on(app, auth).
  defined.  (1 in session)
plg> ?- depends_on(app, X).
  X = auth .
```

(Plus `:`-prefixed [commands](#commands), covered below.)

## Defining your program

Type facts, rules, and directives as you would write them in a file:

```
plg> depends_on(app, ui).
  defined.  (2 in session)
plg> needs(X, Y) :- depends_on(X, Y).
  defined.  (3 in session)
plg> :- dynamic(extra/1).
  defined.  (4 in session)
```

Each accepted entry recompiles the program and confirms with the running
clause count. The session is **saved per-directory** and restored on the
next `plgr` start in the same directory — close the REPL, come back later,
your program is still there. (See [Persistence](#persistence).)

A **rule spanning several lines** continues under the `|  `
prompt until a line ends in `.`:

```
plg> needs(X, Y) :-
|      depends_on(X, Z),
|      needs(Z, Y).
  defined.  (5 in session)
```

Multiple clauses for the same predicate accumulate as alternatives — the
knowledge base is immutable, so there is no in-place "redefine." To change
or remove a clause, edit the whole session with [`:edit`](#commands).

> **Capitalization matters.** A name starting with a capital letter is a
> *variable*, so `Needs(app, X) :- ...` is not a clause — the REPL says so
> and suggests `needs`. Predicate and atom names start lowercase.

## Running queries

A `?- Goal.` query runs against the current program and reveals solutions
**one at a time**, SWI-style:

```
plg> ?- depends_on(app, X).
  X = auth ;
```

The trailing `;` means "there may be more." Press **`;`** or **space** to
see the next solution; press **any other key** (Enter, `.`, …) to stop. The
last solution ends with `.`:

```
  X = auth ;
  X = ui .
```

A goal with no solutions prints `false.`; a ground goal that succeeds
prints `true.`. Solutions are fetched in batches and the REPL transparently
fetches more as you page past one, up to a cap — a query enumerating
thousands of answers stays responsive.

## Editing and history

The input line is a **vim-style editor** (the `vim-line` crate).

- It starts in **insert** mode — just type. Press **Esc** for **normal**
  mode (motions `h l w b 0 $`, edits `x dd C` …); a dim `-- NORMAL --`
  marker appears bottom-right so you always know the mode. `i`/`a`/etc.
  return to insert.
- **History:** **Up/Down arrows** (any mode) or **`k`/`j`** (normal mode)
  walk previously submitted lines. Your in-progress line is stashed and
  restored if you walk back down past the newest entry. History persists
  across sessions in `$HOME/.local/share/plgr_history`.
- **Completion:** **Tab** completes the word under the cursor against the
  builtins, the stdlib, and the predicates you've defined this session.

## Commands

Commands are single-line and start with `:`. Most have a short alias.

| Command | Alias | Effect |
|---|---|---|
| `:load FILE` | `:l` | Consult a `.pl` file — its clauses are added to the session in order. |
| `:list` | `:ls` | Show the session buffer, one numbered clause per entry. |
| `:edit` | `:e` | Open the whole session in `$EDITOR`; on save it reloads and recompiles. |
| `:save FILE` | | Write the current session buffer to `FILE`. |
| `:reset` | | Clear the session (and its saved state for this directory). |
| `:help` | `:h` | Show the command summary. |
| `:quit` | `:q` | Exit. (Ctrl-D or Ctrl-C also quit.) |

`:edit` is the way to **change or delete** clauses: it hands the buffer to
your editor as a `.pl` file, then atomically reloads it. A syntactically
broken edit is rejected and your previous session is left intact.

## How it works

Defining a clause recompiles the session buffer to a fresh temporary native
binary; a `?-` query re-invokes that current binary. So the cost model is:

- **Editing the program** pays a (small) compile each time.
- **Querying** is free of compilation — it just runs the binary you already
  built.

This is the whole point: the REPL gives an interactive feel by *driving the
compiler*, never by interpreting clauses. A divergent or runaway query is
bounded by a timeout and killed without taking the REPL down with it.

## Persistence

The session buffer — the clauses you've defined — is saved to disk and
restored when you next run `plgr` in the **same directory**. Each project
directory has its own session; there is no global program.

- Save happens on exit; load happens at startup. A restored session
  recompiles so the first query works immediately.
- `:reset` clears the buffer *and* deletes the saved state, so a fresh
  start sticks across restarts.
- `:save FILE` and `:load FILE` are separate, explicit operations — they
don't affect the per-directory session.

Sessions live under `$HOME/.local/share/plgr/sessions/`, one file per
directory. (History lives separately at `$HOME/.local/share/plgr_history`.)

## Configuration

| Variable | Purpose |
|---|---|
| `PLGC` | Path to the `plgc` compiler, if not on `PATH`. |
| `VISUAL` / `EDITOR` | Editor for `:edit` (tries `$VISUAL`, then `$EDITOR`, then `vi`). |
| `PLG_REPL_TIMEOUT` | Seconds before a query subprocess is killed (default `10`). |

History lives at `$HOME/.local/share/plgr_history`.
