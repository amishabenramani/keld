#!/usr/bin/env bash
# rustfmt coverage for `include!`d Rust files (#834): the one copy of this
# check. `cargo fmt --all` follows `mod` items only, so a file pulled in by a
# literal `include!("x.rs")` is invisible to it, and so is every `mod` file
# declared inside that file. This script names each such file (discovered from
# tracked sources, so a new `include!` is covered without editing a list) and
# hands it to rustfmt, which then also follows the `mod` items declared in it.
#
#   tools/fmt_included.sh           format the files in place (`just fmt`)
#   tools/fmt_included.sh --check   fail on drift (`just fmt-check`, hosted CI)
#   tools/fmt_included.sh test      self-test against fixture trees
set -euo pipefail

# Must match the workspace edition in Cargo.toml: rustfmt defaults to 2015 when
# it is given a bare file, and cargo is what normally supplies the edition.
edition=2024

# Print one included file path per line: plain `include!("...rs");` statements
# on their own line. `concat!`/`env!` forms and mentions in comments or string
# literals do not match, and an include that resolves to a missing file is
# left to the compiler.
included_files() {
    git grep -nE '^[[:space:]]*include!\("[^"]+\.rs"\);?[[:space:]]*$' -- '*.rs' |
        sed -E 's#^(.*):[0-9]+:[[:space:]]*include!\("([^"]+)"\).*$#\1\t\2#' |
        while IFS=$'\t' read -r includer target; do
            path="$(dirname "$includer")/$target"
            [[ -f "$path" ]] && printf '%s\n' "$path"
        done | sort -u
}

run() {
    local mode="$1" files
    cd "$(git rev-parse --show-toplevel)"
    files="$(included_files)"
    if [[ -z "$files" ]]; then
        echo "fmt_included: no include!d Rust files found; refusing a vacuous pass." >&2
        return 1
    fi
    local -a args=(--edition "$edition")
    [[ "$mode" == check ]] && args+=(--check)
    # shellcheck disable=SC2046 -- one path per line, no spaces in tracked paths
    rustfmt "${args[@]}" $files
}

if [[ "${1:-}" == test ]]; then
    self="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/$(basename "${BASH_SOURCE[0]}")"
    temp="$(mktemp -d "${TMPDIR:-/tmp}/keld-fmt-included.XXXXXX")"
    trap 'rm -rf "$temp"' EXIT
    cp rustfmt.toml "$temp/" 2>/dev/null || cp "$(dirname "$self")/../rustfmt.toml" "$temp/"
    cd "$temp"
    git init -q .
    mkdir -p crates/demo/src/tests tools
    cp "$self" tools/fmt_included.sh
    printf 'pub fn lib() {}\n#[cfg(test)]\nmod tests;\n' >crates/demo/src/lib.rs
    printf 'include!("tests/split.rs");\n' >crates/demo/src/tests.rs
    printf 'mod nested;\nfn split() {}\n' >crates/demo/src/tests/split.rs
    printf 'fn nested() {}\n' >crates/demo/src/tests/nested.rs
    git add . >/dev/null
    check() { tools/fmt_included.sh --check >"$temp/out" 2>&1; }

    check || { echo "FAIL: formatted fixture was rejected" >&2; cat "$temp/out" >&2; exit 1; }
    # Negative control 1: cargo fmt cannot see the include!d file at all.
    printf 'fn split(   ) {}\n' >>crates/demo/src/tests/split.rs
    if check; then echo "FAIL: drift in an include!d file passed" >&2; exit 1; fi
    # Negative control 2: the `mod` file declared inside it is reached too.
    tools/fmt_included.sh >/dev/null
    check || { echo "FAIL: write mode did not format the include!d file" >&2; exit 1; }
    printf 'fn   nested2() {}\n' >>crates/demo/src/tests/nested.rs
    if check; then echo "FAIL: drift in a mod file under an include!d file passed" >&2; exit 1; fi
    # Negative control 3: no include! at all must not be a silent green.
    printf '// nothing\n' >crates/demo/src/tests.rs
    if check; then echo "FAIL: a tree with no include! passed vacuously" >&2; exit 1; fi
    echo "fmt_included self-test: ok"
    exit 0
fi

case "${1:-}" in
    --check) run check ;;
    "") run write ;;
    *) echo "usage: tools/fmt_included.sh [--check|test]" >&2; exit 2 ;;
esac
