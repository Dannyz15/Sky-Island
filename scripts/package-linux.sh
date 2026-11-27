#!/usr/bin/env bash
set -euo pipefail

# Builds the Linux release binary and packages it with cargo-packager into dist/.
# Default format is AppImage (universal Linux / Arch / Omarchy); pass --formats deb or --formats pacman to build those.

FORMATS="appimage"
ALLOW_DIRTY=0
CHECK_ONLY=0

while (($# > 0)); do
    case "$1" in
        --formats)
            FORMATS="${2:?missing value for --formats}"
            shift 2
            ;;
        --allow-dirty)
            ALLOW_DIRTY=1
            shift
            ;;
        --check-only)
            CHECK_ONLY=1
            shift
            ;;
        -h|--help)
            cat <<'EOF'
Usage: scripts/package-linux.sh [--formats appimage,deb,pacman] [--allow-dirty] [--check-only]
EOF
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 2
            ;;
    esac
done

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo_ver="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
packager_ver="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Packager.toml | head -n 1)"
if [[ -z "$cargo_ver" || "$cargo_ver" != "$packager_ver" ]]; then
    echo "Version mismatch: Cargo.toml has '$cargo_ver' but Packager.toml has '$packager_ver'" >&2
    exit 1
fi
echo "Packaging Sky Island $cargo_ver ($FORMATS)"

if ! command -v cargo-packager >/dev/null 2>&1; then
    echo "cargo-packager is not installed. Run: cargo install cargo-packager --locked" >&2
    exit 1
fi

if [[ "$ALLOW_DIRTY" -eq 0 ]] && command -v git >/dev/null 2>&1; then
    if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
        echo "Working tree has uncommitted changes; commit them or pass --allow-dirty for a local test build." >&2
        exit 1
    fi
fi

if [[ "$CHECK_ONLY" -eq 1 ]]; then
    echo "Checks passed (--check-only: nothing was built)."
    exit 0
fi

cargo test --locked
cargo build --release --locked
cargo packager --config Packager.toml --formats "$FORMATS"

ls -lh dist/
