#!/bin/sh
# Build an optimized Debian package without copying local configuration or logs.
set -eu
umask 022
export LC_ALL=C

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
architecture=${1:-$(dpkg --print-architecture)}
case "$architecture" in
    amd64) target=x86_64-unknown-linux-gnu; machine='Advanced Micro Devices X86-64' ;;
    arm64) target=aarch64-unknown-linux-gnu; machine='AArch64' ;;
    *) printf 'Supported architectures: amd64, arm64\n' >&2; exit 1 ;;
esac
if [ "$#" -gt 1 ]; then
    printf 'Usage: %s [amd64|arm64]\n' "$0" >&2
    exit 1
fi

upstream_version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml | head -1)
package_version=${DEB_VERSION:-$upstream_version-1}
dpkg --validate-version "$package_version"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$project_root/target}
export CARGO_PROFILE_RELEASE_STRIP=symbols
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$project_root=/usr/src/ai-stats --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/usr/src/cargo"
if [ "$architecture" != "$(dpkg --print-architecture)" ]; then
    case "$architecture" in
        arm64)
            export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER:-aarch64-linux-gnu-gcc}
            export CC_aarch64_unknown_linux_gnu=${CC_aarch64_unknown_linux_gnu:-aarch64-linux-gnu-gcc}
            ;;
        amd64)
            export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-x86_64-linux-gnu-gcc}
            export CC_x86_64_unknown_linux_gnu=${CC_x86_64_unknown_linux_gnu:-x86_64-linux-gnu-gcc}
            ;;
    esac
fi
cargo build --release --locked --target "$target"
binary="$CARGO_TARGET_DIR/$target/release/ai-stats"
readelf -h "$binary" | grep -F "$machine" >/dev/null

# Fail if a new shared dependency needs to be described in package metadata.
needed=$(readelf -d "$binary" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p')
for library in $needed; do
    case "$library" in
        libc.so.6|libm.so.6|libgcc_s.so.1|libpthread.so.0|libdl.so.2|librt.so.1|ld-linux-x86-64.so.2|ld-linux-aarch64.so.1) ;;
        *) printf 'Unmapped shared dependency: %s\n' "$library" >&2; exit 1 ;;
    esac
done
# Derive the minimum glibc version from the actual binary, including cross builds.
glibc_version=$(readelf --version-info "$binary" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -1)
[ -n "$glibc_version" ]

package_work=$(mktemp -d)
trap 'rm -rf "$package_work"' EXIT HUP INT TERM
stage="$package_work/package"
install -d "$stage/DEBIAN" "$stage/usr/bin" "$stage/usr/share/doc/ai-stats/fonts"
install -m 755 "$binary" "$stage/usr/bin/ai-stats"
install -m 644 LICENSE "$stage/usr/share/doc/ai-stats/copyright"
install -m 644 assets/fonts/*OFL.txt "$stage/usr/share/doc/ai-stats/fonts/"
install -d "$stage/usr/share/man/man1" "$stage/usr/share/lintian/overrides"
gzip -n -9 -c contrib/ai-stats.1 > "$stage/usr/share/man/man1/ai-stats.1.gz"
# These two policy exceptions are intentional for this independent repository.
cat > "$stage/usr/share/lintian/overrides/ai-stats" <<'OVERRIDES'
# serde_yaml embeds its Rust libyaml backend; it does not link system libyaml.
ai-stats: embedded-library libyaml [usr/bin/ai-stats]
# Only the project identity is published; no personal contact data is embedded.
ai-stats: malformed-contact Maintainer AI-Stats
OVERRIDES
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    SOURCE_DATE_EPOCH=$(git log -1 --format=%ct 2>/dev/null || date +%s)
    export SOURCE_DATE_EPOCH
fi
changelog_date=$(LC_ALL=C date -u -d "@$SOURCE_DATE_EPOCH" '+%a, %d %b %Y %H:%M:%S +0000')
{
    printf 'ai-stats (%s) zerotrust; urgency=medium\n\n' "$package_version"
    printf '  * Build AI-Stats from source for %s.\n' "$architecture"
    printf '  * Include the executable, documentation, and license notices only.\n\n'
    printf ' -- AI-Stats contributors <>  %s\n' "$changelog_date"
} | gzip -n -9 > "$stage/usr/share/doc/ai-stats/changelog.Debian.gz"
installed_size=$(du -sk "$stage/usr" | cut -f1)
cat > "$stage/DEBIAN/control" <<CONTROL
Package: ai-stats
Version: $package_version
Architecture: $architecture
Section: utils
Priority: optional
Maintainer: AI-Stats contributors
Installed-Size: $installed_size
Depends: ca-certificates, libc6 (>= $glibc_version), libgcc-s1 (>= 3.0)
Homepage: https://thugs.red
Description: PISG-inspired statistics for AI chats
 Local Claude Code and Codex chat statistics, offline HTML dashboards,
 and AI usage and cost reports with provider sync and CSV/JSON imports.
CONTROL
(
    cd "$stage"
    find usr -type f -print0 | LC_ALL=C sort -z | xargs -0 md5sum > DEBIAN/md5sums
)
find "$stage" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
package_output=${AI_STATS_DEB_OUTPUT_DIR:-$project_root/dist}
mkdir -p "$package_output"
package_path="$package_output/ai-stats_${package_version}_${architecture}.deb"
dpkg-deb --build --root-owner-group -Zxz "$stage" "$package_path"
printf 'Built %s (requires glibc >= %s)\n' "$package_path" "$glibc_version"
