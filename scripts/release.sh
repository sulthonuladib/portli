#!/usr/bin/env bash
#
# release.sh - bump the crate version, tag it, and push.
#
# Pushing the tag triggers .github/workflows/release.yml, which builds the
# binaries and publishes the GitHub Release automatically.
#
# Usage:
#   ./scripts/release.sh patch            # 0.1.0 -> 0.1.1
#   ./scripts/release.sh minor            # 0.1.0 -> 0.2.0
#   ./scripts/release.sh major            # 0.1.0 -> 1.0.0
#   ./scripts/release.sh 1.2.3            # explicit version
#   ./scripts/release.sh 1.2.3-rc.1       # explicit pre-release
#   ./scripts/release.sh                  # interactive prompt (choose major/minor/patch)
#
# Flags:
#   -n, --dry-run   Show what would happen, change nothing.
#   -y, --yes       Skip the confirmation prompt.
#       --no-push   Commit and tag locally, but do not push.
#   -h, --help      Show this help.
#
set -euo pipefail

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------
BRANCH="${RELEASE_BRANCH:-}"        # required branch, empty = any branch
REMOTE="${RELEASE_REMOTE:-origin}"
TAG_PREFIX="v"
CARGO_TOML="Cargo.toml"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
c_red()   { printf '\033[31m%s\033[0m' "$1"; }
c_green() { printf '\033[32m%s\033[0m' "$1"; }
c_dim()   { printf '\033[2m%s\033[0m' "$1"; }

info()  { printf '%s %s\n' "$(c_dim '›')" "$1"; }
ok()    { printf '%s %s\n' "$(c_green '✔')" "$1"; }
die()   { printf '%s %s\n' "$(c_red '✖')" "$1" >&2; exit 1; }

usage() {
    cat <<'EOF'
release.sh - bump the crate version, tag it, and push.

Pushing the tag triggers .github/workflows/release.yml, which builds the
binaries and publishes the GitHub Release automatically.

Usage:
  ./scripts/release.sh patch            # 0.1.0 -> 0.1.1
  ./scripts/release.sh minor            # 0.1.0 -> 0.2.0
  ./scripts/release.sh major            # 0.1.0 -> 1.0.0
  ./scripts/release.sh 1.2.3            # explicit version
  ./scripts/release.sh 1.2.3-rc.1       # explicit pre-release
  ./scripts/release.sh                  # interactive prompt (choose major/minor/patch)

Flags:
  -n, --dry-run   Show what would happen, change nothing.
  -y, --yes       Skip the confirmation prompt.
      --no-push   Commit and tag locally, but do not push.
  -h, --help      Show this help.
EOF
    exit "${1:-0}"
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

# Read the version of the [package] section in Cargo.toml.
read_version() {
    awk '
        /^\[package\][[:space:]]*$/ { in_pkg = 1; next }
        /^\[/                       { in_pkg = 0 }
        in_pkg && $1 == "version" {
            gsub(/["[:space:]]/, "", $3); print $3; exit
        }
    ' "$CARGO_TOML"
}

# Replace the [package] version in Cargo.toml (keeps everything else intact).
write_version() {
    local new="$1" tmp
    tmp="$(mktemp)"
    awk -v new="$new" '
        /^\[package\][[:space:]]*$/ { in_pkg = 1; print; next }
        /^\[/                       { in_pkg = 0 }
        in_pkg && !done && $1 == "version" {
            print "version = \"" new "\""
            done = 1
            next
        }
        { print }
    ' "$CARGO_TOML" > "$tmp"
    mv "$tmp" "$CARGO_TOML"
}

# Compute the next version.
#   bump_version <current> <major|minor|patch|explicit>
bump_version() {
    local cur="$1" kind="$2"
    local base="${cur%%-*}"           # strip pre-release (1.2.3-rc.1 -> 1.2.3)
    local major minor patch

    IFS='.' read -r major minor patch <<< "$base"
    [[ "$major" =~ ^[0-9]+$ && "$minor" =~ ^[0-9]+$ && "$patch" =~ ^[0-9]+$ ]] \
        || die "could not parse current version '$cur'"

    case "$kind" in
        major) echo "$((major + 1)).0.0" ;;
        minor) echo "${major}.$((minor + 1)).0" ;;
        patch) echo "${major}.${minor}.$((patch + 1))" ;;
        *)
            if [[ "$kind" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.+][0-9A-Za-z.-]+)?$ ]]; then
                echo "$kind"
            else
                die "invalid version or bump type: '$kind' (use major|minor|patch|X.Y.Z)"
            fi
            ;;
    esac
}

# ---------------------------------------------------------------------------
# Arguments
# ---------------------------------------------------------------------------
DRY_RUN=0
ASSUME_YES=0
PUSH=1
KIND=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n|--dry-run) DRY_RUN=1 ;;
        -y|--yes)     ASSUME_YES=1 ;;
        --no-push)    PUSH=0 ;;
        -h|--help)    usage 0 ;;
        major|minor|patch) KIND="$1" ;;
        -*)           die "unknown option: $1 (try --help)" ;;
        *)
            if [[ -z "$KIND" && "$1" =~ ^[0-9] ]]; then
                KIND="$1"
            else
                die "unexpected argument: $1 (try --help)"
            fi
            ;;
    esac
    shift
done

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------
[[ -f "$CARGO_TOML" ]] || die "run this from the repository root ($CARGO_TOML not found)"
require_cmd git
require_cmd awk
require_cmd mktemp

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || die "not a git repository"

HAS_REMOTE=0
git remote get-url "$REMOTE" >/dev/null 2>&1 && HAS_REMOTE=1

CURRENT="$(read_version)"
[[ -n "$CURRENT" ]] || die "could not read version from $CARGO_TOML"

CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ -n "$BRANCH" && "$CURRENT_BRANCH" != "$BRANCH" ]]; then
    die "must release from branch '$BRANCH' (currently on '$CURRENT_BRANCH')"
fi

if [[ "$DRY_RUN" -eq 0 ]] && [[ -n "$(git status --porcelain)" ]]; then
    die "working tree is dirty; commit or stash your changes first"
fi

if [[ "$PUSH" -eq 1 && "$DRY_RUN" -eq 0 && "$HAS_REMOTE" -eq 0 ]]; then
    die "git remote '$REMOTE' not found; add one (git remote add $REMOTE <url>) or pass --no-push"
fi

# ---------------------------------------------------------------------------
# Choose the bump type interactively when not supplied
# ---------------------------------------------------------------------------
if [[ -z "$KIND" ]]; then
    if [[ ! -t 0 ]]; then
        die "no bump type given (use major|minor|patch|X.Y.Z)"
    fi
    base="${CURRENT%%-*}"
    IFS='.' read -r _M _m _p <<< "$base"
    printf 'Current version: %s\n\n' "$(c_green "$CURRENT")"
    printf '  1) major  -> %s\n' "$((_M + 1)).0.0"
    printf '  2) minor  -> %s\n' "${_M}.$((_m + 1)).0"
    printf '  3) patch  -> %s\n\n' "${_M}.${_m}.$((_p + 1))"
    read -r -p 'Select bump [1/2/3]: ' choice
    case "$choice" in
        1|major) KIND=major ;;
        2|minor) KIND=minor ;;
        3|patch) KIND=patch ;;
        *) die "invalid selection: '$choice'" ;;
    esac
fi

NEW="$(bump_version "$CURRENT" "$KIND")"
TAG="${TAG_PREFIX}${NEW}"

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && die "tag '$TAG' already exists"
if [[ "$HAS_REMOTE" -eq 1 ]] && git ls-remote --exit-code --tags "$REMOTE" "refs/tags/$TAG" >/dev/null 2>&1; then
    die "tag '$TAG' already exists on '$REMOTE'"
fi

# ---------------------------------------------------------------------------
# Summary / confirmation
# ---------------------------------------------------------------------------
printf '\n'
info "current version : $CURRENT"
info "new version     : $(c_green "$NEW")"
info "tag             : $TAG"
info "branch          : $CURRENT_BRANCH"
info "remote          : $REMOTE"
info "push            : $([[ "$PUSH" -eq 1 ]] && echo yes || echo no)"
[[ "$DRY_RUN" -eq 1 ]] && info "mode            : $(c_red DRY RUN)"
printf '\n'

if [[ "$ASSUME_YES" -eq 0 && "$DRY_RUN" -eq 0 ]]; then
    read -r -p "Release $TAG? [y/N] " reply
    [[ "$reply" =~ ^[Yy]$ ]] || die "aborted"
fi

# ---------------------------------------------------------------------------
# Do it
# ---------------------------------------------------------------------------
if [[ "$DRY_RUN" -eq 1 ]]; then
    ok "dry run complete - nothing was changed"
    exit 0
fi

info "updating $CARGO_TOML"
write_version "$NEW"

# Keep Cargo.lock in sync with the new crate version.
if command -v cargo >/dev/null 2>&1; then
    info "refreshing Cargo.lock"
    cargo update --workspace --offline >/dev/null 2>&1 \
        || cargo metadata --format-version 1 >/dev/null 2>&1 \
        || info "warning: could not refresh Cargo.lock, continuing"
fi

git add "$CARGO_TOML"
[[ -f Cargo.lock ]] && git add Cargo.lock

info "committing"
git commit -m "chore(release): ${TAG}" >/dev/null

info "tagging ${TAG}"
git tag -a "$TAG" -m "Release ${TAG}"

if [[ "$PUSH" -eq 1 ]]; then
    info "pushing ${CURRENT_BRANCH} and ${TAG} to ${REMOTE}"
    git push "$REMOTE" "$CURRENT_BRANCH"
    git push "$REMOTE" "$TAG"
    printf '\n'
    ok "released ${TAG#v} - the Release workflow will publish the GitHub release shortly"
    info "watch it with: gh run watch"
else
    printf '\n'
    ok "committed and tagged ${TAG} locally (not pushed)"
    info "push with: git push $REMOTE $CURRENT_BRANCH && git push $REMOTE $TAG"
fi
