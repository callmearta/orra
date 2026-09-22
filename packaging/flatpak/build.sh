#!/usr/bin/env bash
#
# Build Orra as a Flatpak, and optionally publish the result as a repository
# other machines can install from.
#
#     ./build.sh                 # build and install it for this user
#     ./build.sh --repo          # build, and export a repo + .flatpakref to publish
#     ./build.sh --no-build --repo   # package binaries that already exist (CI)
#
# Run from anywhere; it works on the directory it lives in. Unlike the AUR
# package this does not build the app from a released tarball — the binaries
# the release workflow produces are staged and packaged, which is why the
# version comes from tauri.conf.json rather than from an argument.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app_id=ai.orra.desktop
manifest="$here/$app_id.yml"
stage="$here/stage"

version="$(node -p "require('$root/src-tauri/tauri.conf.json').version")"
echo "building $app_id $version"

command -v flatpak-builder >/dev/null || {
  echo "flatpak-builder is missing. Install it (flatpak-builder, or the" >&2
  echo "org.flatpak.Builder app) and try again." >&2
  exit 1
}

no_build=false
for arg in "$@"; do
  [ "$arg" = "--no-build" ] && no_build=true
done

if [ "$no_build" = false ]; then
  # The UI is compiled into the binary, so it has to exist before cargo runs.
  echo "building the UI"
  (cd "$root/ui" && npm ci >/dev/null && npm run build)

  echo "building the app"
  (cd "$root/src-tauri" && cargo build --release --locked)
fi

echo "staging"
rm -rf "$stage"
mkdir -p "$stage"
install -m755 "$root/src-tauri/target/release/orra" "$stage/orra"
install -m755 "$root/src-tauri/target/release/orra-ctl" "$stage/orra-ctl"
for size in 32x32 64x64 128x128 256x256; do
  install -m644 "$root/src-tauri/icons/$size.png" "$stage/$size.png"
done

# A runtime has to be somewhere flatpak-builder can see it, and which
# installation that is decides whether `--user` belongs on the command line.
# Building against the system one needs no root and no download when it is
# already there — which on a machine that runs Flatpak apps it usually is —
# while a user-only setup needs `--user` or flatpak-builder will not find the
# SDK it was told to use.
if flatpak --user list --runtime 2>/dev/null | grep -q "org.gnome.Sdk"; then
  runtime_flag=(--user)
elif flatpak list --runtime 2>/dev/null | grep -q "org.gnome.Sdk"; then
  runtime_flag=()
else
  echo "org.gnome.Sdk//50 is not installed, in either the user or the system" >&2
  echo "installation. Install it first, either way round:" >&2
  echo "  flatpak install --user flathub org.gnome.Sdk//50 org.gnome.Platform//50" >&2
  echo "  sudo flatpak install flathub org.gnome.Sdk//50 org.gnome.Platform//50" >&2
  exit 1
fi

echo "building the flatpak"
flatpak-builder --force-clean "${runtime_flag[@]}" \
  --repo="$here/repo" --default-branch=stable "$here/build" "$manifest"

if [[ " $* " == *" --repo "* ]]; then
  # A summary file is what lets `flatpak install` show anything useful. The
  # repo is unsigned, so installing from it needs --no-gpg-verify; a published
  # one wants a GPG key, which is the difference between this and Flathub.
  flatpak build-update-repo --generate-static-deltas "$here/repo"

  # A single-file bundle is the install story: one download, no repository to
  # host, and a desktop that offers to install it when you open it. The repo
  # stays beside it for anyone who would rather point a client at a URL — set
  # --repo-url to wherever that ends up being hosted, and updates follow.
  flatpak build-bundle "$here/repo" "$here/$app_id.flatpak" "$app_id" stable
  echo "bundle: $here/$app_id.flatpak"
  echo "repo:   $here/repo (for a hosted remote; update --repo-url to match)"
else
  flatpak --user remote-add --if-not-exists --no-gpg-verify orra "$here/repo" 2>/dev/null || true
  flatpak --user install -y orra "$app_id"
  echo "installed. run it with: flatpak run $app_id"
fi
