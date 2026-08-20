#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./make_release.sh VERSION

Create a release from the current JJ checkout. VERSION must be a Cargo-style
semantic version without a leading "v" (for example, 0.2.0 or 1.0.0-rc.1).

The script updates Cargo.toml and Cargo.lock, runs the release checks, pushes
the main bookmark with JJ, and creates the vVERSION tag and GitHub Release with
the GitHub CLI. The tag triggers .github/workflows/release.yml.
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

if [[ ${1:-} == "-h" || ${1:-} == "--help" ]]; then
  usage
  exit 0
fi

[[ $# -eq 1 ]] || {
  usage >&2
  exit 2
}

release_version=$1
[[ $release_version != v* ]] || die 'pass the version without a leading "v"'
[[ $release_version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] || \
  die "invalid semantic version: $release_version"

release_tag="v$release_version"

for command_name in cargo gh jj perl; do
  command -v "$command_name" >/dev/null 2>&1 || die "required command not found: $command_name"
done

repo_root=$(jj workspace root)
cd "$repo_root"

[[ -f Cargo.toml && -f Cargo.lock ]] || die 'run this script from the game repository'
[[ -z $(jj diff --summary) ]] || die 'the JJ working copy has uncommitted changes'
[[ -n $(jj log -r '@ & main::' --no-graph -T 'commit_id' 2>/dev/null) ]] || \
  die 'the current JJ change is not based on the main bookmark'

gh auth status >/dev/null
if gh release view "$release_tag" >/dev/null 2>&1; then
  die "GitHub Release already exists: $release_tag"
fi
if gh api "repos/{owner}/{repo}/git/ref/tags/$release_tag" >/dev/null 2>&1; then
  die "Git tag already exists on GitHub: $release_tag"
fi

current_version=$(cargo metadata --no-deps --format-version 1 \
  | perl -ne 'print "$1\n" if /"name":"toy-hover-battle","version":"([^"]+)"/')
[[ -n $current_version ]] || die 'could not read the toy-hover-battle package version'
[[ $current_version != "$release_version" ]] || die "Cargo package is already version $release_version"

printf 'Preparing %s (currently %s)\n' "$release_tag" "$current_version"

# The checkout is required to be empty above, so reuse it for the version bump.
# Creating a child here would leave the current empty change as an undescribed
# commit in the release history.
jj describe -m "Release $release_tag"

export TOY_RELEASE_VERSION=$release_version
perl -0pi -e \
  's/(\[package\]\s+name\s*=\s*"toy-hover-battle"\s+version\s*=\s*")[^"]+("\s+)/$1$ENV{TOY_RELEASE_VERSION}$2/' \
  Cargo.toml

# Cargo refreshes the root package entry in Cargo.lock without changing the
# selected dependency versions.
cargo check --locked >/dev/null 2>&1 && \
  die 'Cargo.lock unexpectedly accepted the old package version'
cargo check

grep -Fq "version = \"$release_version\"" Cargo.toml || die 'Cargo.toml version update failed'
lock_version=$(perl -0ne \
  'print "$1\n" if /\[\[package\]\]\s+name = "toy-hover-battle"\s+version = "([^"]+)"/' \
  Cargo.lock)
[[ $lock_version == "$release_version" ]] || die 'Cargo.lock version update failed'

cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release --locked

jj bookmark set main -r @
release_commit=$(jj log -r @ --no-graph -T 'commit_id')

printf 'Pushing main at %s with JJ\n' "$release_commit"
jj git push --bookmark main

release_args=(
  release create "$release_tag"
  --target "$release_commit"
  --title "Toy Hover Battle $release_tag"
  --generate-notes
)
if [[ $release_version == *-* ]]; then
  release_args+=(--prerelease)
fi

printf 'Creating GitHub Release %s\n' "$release_tag"
gh "${release_args[@]}"

# Leave a clean child change ready for subsequent development while the main
# bookmark remains on the tagged release commit.
jj new

printf '\nRelease %s created. Follow the platform builds with:\n' "$release_tag"
printf '  gh run list --workflow release.yml --limit 1\n'
