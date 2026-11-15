#!/bin/sh
# Builds the release archive of femp for one target, in the layout the
# installer unpacks into FEMP_HOME:
#
#   femp-<target>/bin/femp
#   femp-<target>/toolchains/fe-<version>/bin/fe
#   femp-<target>/LICENSE, LICENSE-fe
#
#   install/build-archive.sh <target> [out-dir]
#
# The Fe compiler is built from its release tag with every backend femp
# drives, which needs Rust 1.97 or newer and npm. To package a compiler that
# is already built instead, set FEMP_ARCHIVE_FE to it and FEMP_ARCHIVE_FE_LICENSE
# to its license file. FEMP_ARCHIVE_RUST names a rustup toolchain to build with.
set -eu

[ $# -ge 1 ] || {
    echo "usage: $0 <target> [out-dir]" >&2
    exit 2
}
target=$1
out=${2:-out}
root=$(cd "$(dirname "$0")/.." && pwd)
fe_repository="https://github.com/argotorg/fe"
fe_version=$(sed -n 's/^pub const DEFAULT_FE_VERSION: &str = "\(.*\)";$/\1/p' \
    "$root/crates/femp-core/src/manifest.rs")
[ -n "$fe_version" ] || {
    echo "cannot read the pinned Fe version" >&2
    exit 1
}

cargo() {
    command cargo ${FEMP_ARCHIVE_RUST:+"+$FEMP_ARCHIVE_RUST"} "$@"
}

name="femp-$target"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
package="$stage/$name"
toolchain="$package/toolchains/fe-$fe_version/bin"
mkdir -p "$package/bin" "$toolchain"

cargo build --release --locked -p femp-cli --target "$target" --manifest-path "$root/Cargo.toml"
cp "${CARGO_TARGET_DIR:-$root/target}/$target/release/femp" "$package/bin/femp"

if [ -n "${FEMP_ARCHIVE_FE:-}" ]; then
    cp "$FEMP_ARCHIVE_FE" "$toolchain/fe"
    cp "${FEMP_ARCHIVE_FE_LICENSE:?set FEMP_ARCHIVE_FE_LICENSE to the Fe license file}" \
        "$package/LICENSE-fe"
else
    git clone --quiet --depth 1 --branch "v$fe_version" "$fe_repository" "$stage/fe"
    # The grammar crate generates Fe's parser with the tree-sitter CLI it pins.
    (cd "$stage/fe/crates/tree-sitter-fe" && npm ci --ignore-scripts && npm rebuild tree-sitter-cli)
    cargo install --locked --features cranelift,wasm --target "$target" \
        --path "$stage/fe/crates/fe" --root "$stage/fe-install"
    cp "$stage/fe-install/bin/fe" "$toolchain/fe"
    cp "$stage/fe/LICENSE-APACHE" "$package/LICENSE-fe"
fi
cp "$root/LICENSE" "$package/LICENSE"

mkdir -p "$out"
tar -czf "$out/$name.tar.gz" -C "$stage" "$name"
echo "$out/$name.tar.gz"
