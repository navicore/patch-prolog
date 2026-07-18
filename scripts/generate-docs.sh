#!/bin/bash
# Generate docs/README.md from the repo-root README.md.
#
# docs/README.md is the mdBook introduction page. It is 100% GENERATED —
# never edit or commit it. Edits belong in the root README.md; content that
# must not appear in the book goes inside docs:skip markers there:
#
#   <!-- docs:skip-start -->
#   ...repo-only content...
#   <!-- docs:skip-end -->
#
# Repairs applied for the mdbook context:
#   - drop badge/shield lines (lines starting with `[![`)
#   - drop docs:skip-start .. docs:skip-end regions (and the markers)
#   - rewrite repo-relative `](docs/…)` links to book-relative `](…)`
#   - convert `## ` headers to bold so they don't clutter the mdBook sidebar
#
# If the output needs a hand edit, fix this script or the markers in
# README.md — never the output.
set -euo pipefail

out=$(mktemp)

awk '
  /<!-- *docs:skip-start *-->/ { skip=1; next }
  /<!-- *docs:skip-end *-->/   { skip=0; next }
  skip                         { next }
  /^\[!\[/                     { next }   # badge lines
  {
    gsub(/\]\(docs\//, "](")              # link targets: ](docs/x.md) -> ](x.md)
    gsub(/\[docs\//, "[")                 # link text:    [docs/x.md] -> [x.md]
    if ($0 ~ /^## /) {                    # ## H -> **H**
      print "**" substr($0, 4) "**"
    } else {
      print
    }
  }
' README.md > "$out"

# Fail loudly if a docs/ link survived the rewrite (e.g. split across lines).
if grep -n '](docs/' "$out"; then
  echo "error: unrewritten docs/ link in generated docs/README.md" >&2
  rm -f "$out"
  exit 1
fi

{
  echo "<!-- GENERATED from README.md by scripts/generate-docs.sh — do not edit -->"
  echo
  cat "$out"
} > docs/README.md
rm -f "$out"
