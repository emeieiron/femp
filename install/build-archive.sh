#!/bin/sh
# Builds the release archive of femp for one target, in the layout the
# installer unpacks into FEMP_HOME:
#
#   femp-<target>/bin/femp
#   femp-<target>/z3/bin/z3
#   femp-<target>/LICENSE, LICENSE-fe, LICENSE-z3
#
#   install/build-archive.sh <target> [out-dir]
#
# femp links the Fe compiler, so the archive holds no compiler of its own;
# LICENSE-fe is the license of the compiler crates femp is built with.
# FEMP_ARCHIVE_RUST names a rustup toolchain to build with (Rust 1.97 or
# newer).
#
# The Z3 solver `femp prove` drives is a pinned release build, checked
# against the SHA-256 below before it is packaged.
set -eu

z3_version="5.4.1"

[ $# -ge 1 ] || {
    echo "usage: $0 <target> [out-dir]" >&2
    exit 2
}
target=$1
out=${2:-out}
root=$(cd "$(dirname "$0")/.." && pwd)

cargo() {
    command cargo ${FEMP_ARCHIVE_RUST:+"+$FEMP_ARCHIVE_RUST"} "$@"
}

name="femp-$target"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
package="$stage/$name"
mkdir -p "$package/bin"

cargo build --release --locked -p femp-cli --target "$target" --manifest-path "$root/Cargo.toml"
cp "${CARGO_TARGET_DIR:-$root/target}/$target/release/femp" "$package/bin/femp"
cp "$root/LICENSE" "$package/LICENSE"
# The license of the compiler crates, from the checkout Cargo made of their tag.
fe_license=$(cargo metadata --locked --format-version 1 --manifest-path "$root/Cargo.toml" |
    python3 -c '
import json, os, sys
for package in json.load(sys.stdin)["packages"]:
    if package["name"] == "fe-codegen":
        crate = os.path.dirname(package["manifest_path"])
        print(os.path.normpath(os.path.join(crate, "..", "..", "LICENSE-APACHE")))
        break
')
[ -f "$fe_license" ] || {
    echo "cannot find the license of the Fe compiler crates" >&2
    exit 1
}
cp "$fe_license" "$package/LICENSE-fe"

case "$target" in
    x86_64-unknown-linux-gnu)
        z3_build="x64-glibc-2.39"
        z3_sha256="b548bac48ef2452b4e09ea2945cf4b011d5f43fcf39e4799df268ee7e0769d85"
        ;;
    aarch64-unknown-linux-gnu)
        z3_build="arm64-glibc-2.38"
        z3_sha256="9e8f781f15a423ede5bd491b665966dbb607d9f9c4f9f9b502116f271d1b050f"
        ;;
    aarch64-apple-darwin)
        z3_build="arm64-osx-13.3"
        z3_sha256="a4a878121e81dfc297d027dd0b51dc4f08feb4296d6ecace42bfbc87a29d6d58"
        ;;
    x86_64-apple-darwin)
        z3_build="x64-osx-13.3"
        z3_sha256="79e93b0564b82bf2c1b355df130bab543b1ff88d597c985632770fea05f1b249"
        ;;
    *)
        echo "no Z3 build is pinned for $target" >&2
        exit 1
        ;;
esac
z3_name="z3-$z3_version-$z3_build"
curl -fsSL -o "$stage/$z3_name.zip" \
    "https://github.com/Z3Prover/z3/releases/download/z3-$z3_version/$z3_name.zip"
z3_actual=$( (sha256sum "$stage/$z3_name.zip" 2>/dev/null || shasum -a 256 "$stage/$z3_name.zip") | cut -d' ' -f1)
[ "$z3_actual" = "$z3_sha256" ] || {
    echo "$z3_name.zip has SHA-256 $z3_actual, expected $z3_sha256" >&2
    exit 1
}
unzip -q "$stage/$z3_name.zip" -d "$stage"
mkdir -p "$package/z3/bin"
cp "$stage/$z3_name/bin/z3" "$package/z3/bin/z3"
for library in "$stage/$z3_name"/bin/libz3.*; do
    [ -e "$library" ] && cp "$library" "$package/z3/bin/"
done
cp "$stage/$z3_name/LICENSE.txt" "$package/LICENSE-z3"

mkdir -p "$out"
tar -czf "$out/$name.tar.gz" -C "$stage" "$name"
echo "$out/$name.tar.gz"
