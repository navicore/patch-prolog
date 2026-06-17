# LSP & Editor Guide

`plgl` is a Language Server for the patch-prolog language. It gives any
LSP-capable editor real-time diagnostics, completion, hover docs, and
go-to-definition over stdio. Unlike `plgc`, the server links only the
parser — **no clang, no runtime** — so it's small and starts instantly.

## What it provides

- **Diagnostics** — syntax/parse errors as you type, plus **warnings for
  calls to predicates defined nowhere**, with did-you-mean suggestions
  (e.g. a typo'd `parnet/2` suggests `parent/2`). These are warnings in the
  editor; to make them hard build failures, use `plgc --deny-undefined`
  (see [Compiler Usage](compiler-usage.md#undefined-predicates)).
- **Completion** — builtins, the stdlib, and the predicates defined in the
  current buffer.
- **Hover** — a one-line doc for builtins, and the clause head for
  user-defined predicates.
- **Go-to-definition** — jumps to a predicate's first clause.

Completion, hover, and the undefined-predicate check all draw on the same
sources as the compiler (the shared builtin table, the stdlib, the
frontend parser), so the editor agrees with what `plgc` accepts.

## Installing `plgl`

The toolchain isn't on crates.io yet, so build from source. The server
needs only the parser, so you can install just it:

```sh
git clone https://git.navicore.tech/navicore/patch-prolog
cd patch-prolog
cargo install --path crates/lsp
```

Or install the whole toolchain at once:

```sh
just install        # plgc, plgr, plgl
```

Verify with `plgl --version`. The server speaks LSP over stdio; you don't
run it directly — your editor launches it.

## Neovim

The companion plugin
[`patch-prolog.nvim`](https://git.navicore.tech/navicore/patch-prolog.nvim)
wires up the LSP along with filetype detection (`.pl`, `.plg`, `.pro` →
`prolog`), syntax highlighting, and indentation. It needs Neovim 0.10+ and
`plgl` on your `PATH`.

With [lazy.nvim](https://github.com/folke/lazy.nvim):

```lua
{
  "navicore/patch-prolog.nvim",
  ft = "prolog",
  opts = {},
}
```

Or configure it directly:

```lua
require("plgl").setup({
  -- cmd = { "plgl" },              -- override the binary path/args if needed
  on_attach = function(client, bufnr)
    local map = vim.keymap.set
    map("n", "gd", vim.lsp.buf.definition, { buffer = bufnr })
    map("n", "K",  vim.lsp.buf.hover,      { buffer = bufnr })
  end,
})
```

> Neovim's built-in detection maps `.pl` to Perl; the plugin overrides that
> to `prolog`.

## Any other LSP client

`plgl` is a standard stdio language server — any client can drive it with
three things:

- **Command:** `plgl` (no arguments; communicates over stdin/stdout).
- **Language / filetype:** `prolog`.
- **Root directory:** the project root — `.git` is the usual marker.

For example, a bare Neovim client without the plugin:

```lua
vim.lsp.start({
  name = "plgl",
  cmd = { "plgl" },
  root_dir = vim.fs.root(0, { ".git", "Cargo.toml" }),
})
```

The same three parameters translate to VS Code, Emacs `eglot`/`lsp-mode`,
Helix (`languages.toml`), and others.

## Known limitations

- Hover and go-to-definition work on **identifier names**. Operator-named
  builtins (`=..`, `@<`, `\+`) aren't hoverable — the cursor-word
  extraction only spans identifier characters.
- Diagnostics are computed per file from the open buffer; cross-file
  project resolution is not modeled (a predicate defined in another file
  the buffer doesn't include reads as undefined).
