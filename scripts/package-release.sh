#!/bin/bash
# Builds, checks and packages the macOS release binaries into dist/: the same steps run
# locally and in .github/workflows/release.yml, which only adds the publishing.
# Usage: scripts/package-release.sh <tag>, where <tag> is v<version> or v<version>-<suffix>.
set -euo pipefail

die() {
  echo "$*" >&2
  exit 1
}

[ $# -eq 1 ] || die "usage: scripts/package-release.sh <tag>"
tag=$1
cd "$(dirname "$0")/.."

version=$(sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[[ "$tag" == "v$version" || "$tag" == "v$version-"* ]] || die "tag $tag does not match Cargo.toml version $version"

targets=(aarch64-apple-darwin x86_64-apple-darwin)
cargo test --locked
rustup target add "${targets[@]}"
for t in "${targets[@]}"; do
  # Without it the C code of tree-sitter targets the macOS of the build machine.
  MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --locked --target "$t"
done

for t in "${targets[@]}"; do
  bin=target/$t/release/kotlin-navigator
  want=${t%%-*}
  [ "$want" = aarch64 ] && want=arm64
  got=$(lipo -archs "$bin")
  [ "$got" = "$want" ] || die "$bin: architecture $got, expected $want"
  vtool -show-build "$bin" | grep -Eq '^ *minos 11\.0$' || die "$bin: not built for macOS 11.0"
done
got=$(target/aarch64-apple-darwin/release/kotlin-navigator --version)
[ "$got" = "kotlin-navigator $version" ] || die "--version printed '$got', expected 'kotlin-navigator $version'"

rm -rf dist
mkdir dist
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
for t in "${targets[@]}"; do
  archive=kotlin-navigator-$t.tar.gz
  cp "target/$t/release/kotlin-navigator" "$stage/kotlin-navigator"
  # No AppleDouble files and no local user name in the archive.
  COPYFILE_DISABLE=1 tar --no-mac-metadata --uid 0 --gid 0 --uname root --gname root -czf "dist/$archive" -C "$stage" kotlin-navigator
  [ "$(tar -tzf "dist/$archive")" = kotlin-navigator ] || die "dist/$archive: expected only kotlin-navigator inside"
  (cd dist && shasum -a 256 "$archive" > "$archive.sha256" && shasum -a 256 -c "$archive.sha256" > /dev/null) || die "dist/$archive: checksum failed"
done
ls dist
