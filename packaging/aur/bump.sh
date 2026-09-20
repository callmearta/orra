#!/usr/bin/env bash
#
# Point the AUR packaging at a released tag and regenerate what the AUR wants.
#
#     ./bump.sh 0.1.4
#
# Run from anywhere; it works on the directory it lives in. The hash is taken
# from the tarball GitHub serves for the tag, which is the same one a user's
# build will download — if the tag is not pushed yet, this fails rather than
# writing a hash for something that does not exist.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
version="${1:?usage: bump.sh <version, without the v>}"
url="https://github.com/callmearta/orra/archive/refs/tags/v${version}.tar.gz"

echo "fetching $url"
sha="$(curl -fsSL "$url" | sha256sum | cut -d' ' -f1)"

cd "$here"
sed -i "s/^pkgver=.*/pkgver=${version}/; s/^pkgrel=.*/pkgrel=1/" PKGBUILD
sed -i "s/^sha256sums=(.*/sha256sums=('${sha}' 'SKIP' 'SKIP')/" PKGBUILD

# Required, and rejected if it disagrees with the PKGBUILD above.
makepkg --printsrcinfo > .SRCINFO

echo "PKGBUILD at ${version}, sha256 ${sha}"
echo "now copy PKGBUILD .SRCINFO orra.desktop LICENSE into the AUR clone and push"
