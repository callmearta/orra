#!/bin/sh
# Build the StatusNotifier tray stack into /app, in dependency order.
#
# The GNOME runtime ships none of these, and the tray is not optional: the app
# builds a TrayIcon, and libayatana-appindicator is dlopen'd rather than linked,
# so a runtime without it does not fail loudly — it just has no tray icon.
#
# Kept as a script rather than as three manifest modules so that the local
# build and the one CI runs are the same commands rather than two descriptions
# of the same thing that can drift apart.
set -e

export PKG_CONFIG_PATH=/app/lib/pkgconfig:/app/share/pkgconfig:${PKG_CONFIG_PATH:-}
export LD_LIBRARY_PATH=/app/lib:/app/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH:-}
export PATH=/app/bin:${PATH}
J="$(nproc)"
# flatpak-builder unpacks every source into this one directory, and each
# project has to be entered from it rather than from wherever the last one
# left us.
ROOT="$PWD"

# The three libraries and intltool arrive as plain tar files, so unpack them
# here rather than leaving it to flatpak-builder: it strips an archive's leading
# directory when it unpacks, which for four archives in one module means pouring
# them all into the same place. Each of these unpacks to a directory of its own.
for archive in "$ROOT"/*.tar.gz "$ROOT"/*.tar.xz; do
  [ -f "$archive" ] || continue
  tar -xf "$archive" -C "$ROOT"
done

# libdbusmenu's configure.ac calls IT_PROG_INTLTOOL, and the SDK has no
# intltool — it is long deprecated, which is why it is not there. Only its m4
# macros and helper scripts are wanted, and only for the translation rules in a
# build that installs no translations.
echo "### 0/4 intltool"
cd "$ROOT" && cd "$(ls -d intltool-*/ | head -1)"
./configure --prefix=/app >/tmp/intltool-conf.log 2>&1 || { echo "configure failed"; tail -20 /tmp/intltool-conf.log; exit 1; }
# The tarball ships a tree built by automake 1.14, and unpacking gives
# configure.ac a newer timestamp than the generated files — so make decides to
# regenerate them and dies looking for tools that no longer exist. Restoring
# the order it expects leaves the shipped files alone.
touch aclocal.m4 configure Makefile.in 2>/dev/null || true
make >/tmp/intltool-make.log 2>&1 || { echo "make failed"; tail -20 /tmp/intltool-make.log; exit 1; }
make install >/tmp/intltool-install.log 2>&1 || { echo "install failed"; tail -20 /tmp/intltool-install.log; exit 1; }
# Where aclocal and autoreconf look for third-party macros.
export ACLOCAL_PATH=/app/share/aclocal:${ACLOCAL_PATH:-}
echo "    intltool.m4: $(ls /app/share/aclocal/intltool.m4 2>/dev/null || echo MISSING)"

echo "### 1/4 libdbusmenu"
cd "$ROOT" && cd "$(ls -d libdbusmenu-*/ | head -1)"
# Not autogen.sh: that only wraps this in gnome-autogen.sh, which wants
# gnome-common — not in the SDK — and the GNOME_* macros it exists to provide
# are not used by this configure.ac at all.
# configure.ac declares HAVE_VALGRIND inside the block that only runs when the
# test suite is enabled, so with tests off autoconf stops with "conditional
# HAVE_VALGRIND was never defined" long before it reaches a compiler. Declaring
# it unconditionally is the whole fix — nothing else reads have_valgrind, and
# with tests off it is simply false.
sed -i '/AM_CONDITIONAL(\[HAVE_VALGRIND\]/d' configure.ac
printf '\nAM_CONDITIONAL([HAVE_VALGRIND], [test "x$have_valgrind" = "xyes"])\n' >> configure.ac
autoreconf -fi >/tmp/autoreconf.log 2>&1 || { echo "autoreconf failed"; tail -20 /tmp/autoreconf.log; exit 1; }
# Introspection is off because its typelib is installed to an absolute
# girepository path that ignores --prefix, which is read-only inside the build
# sandbox. The GIR bindings are not used by anything here: the tray only needs
# the shared libraries.
./configure --prefix=/app --with-gtk=3 --disable-vala --disable-tests --disable-dumper \
  --disable-introspection --disable-gtk-doc \
  >/tmp/dbusmenu-conf.log 2>&1 || { echo "configure failed"; tail -30 /tmp/dbusmenu-conf.log; exit 1; }
make -j"$J" >/tmp/dbusmenu-make.log 2>&1 || { echo "make failed"; tail -30 /tmp/dbusmenu-make.log; exit 1; }
make install >/tmp/dbusmenu-install.log 2>&1 || { echo "install failed"; tail -20 /tmp/dbusmenu-install.log; exit 1; }
echo "    installed $(ls /app/lib/*dbusmenu* 2>/dev/null | wc -l) files"

echo "### 2/4 libayatana-indicator"
cd "$ROOT" && cd "$(ls -d libayatana-indicator-*/ | head -1)"
cmake -S . -B _b -DCMAKE_INSTALL_PREFIX=/app -DCMAKE_INSTALL_LIBDIR=lib \
  -DCMAKE_BUILD_TYPE=Release -DFLAVOUR_GTK3=ON -DFLAVOUR_GTK2=OFF -DENABLE_TESTS=OFF -DENABLE_WERROR=OFF \
  -DENABLE_IDO=OFF -DENABLE_LOADER=OFF \
  >/tmp/indicator-conf.log 2>&1 || { echo "configure failed"; tail -30 /tmp/indicator-conf.log; exit 1; }
cmake --build _b -j"$J" >/tmp/indicator-make.log 2>&1 || { echo "build failed"; tail -30 /tmp/indicator-make.log; exit 1; }
cmake --install _b >/tmp/indicator-install.log 2>&1 || { echo "install failed"; tail -20 /tmp/indicator-install.log; exit 1; }
echo "    installed $(ls /app/lib/*ayatana-indicator* 2>/dev/null | wc -l) files"

echo "### 3/4 libayatana-appindicator"
cd "$ROOT" && cd "$(ls -d libayatana-appindicator-*/ | head -1)"
cmake -S . -B _b -DCMAKE_INSTALL_PREFIX=/app -DCMAKE_INSTALL_LIBDIR=lib \
  -DCMAKE_BUILD_TYPE=Release -DFLAVOUR_GTK3=ON -DFLAVOUR_GTK2=OFF -DENABLE_TESTS=OFF -DENABLE_WERROR=OFF \
  -DENABLE_BINDINGS_VALA=OFF -DENABLE_BINDINGS_MONO=OFF -DENABLE_GTKDOC=OFF \
  >/tmp/appindicator-conf.log 2>&1 || { echo "configure failed"; tail -30 /tmp/appindicator-conf.log; exit 1; }
cmake --build _b -j"$J" >/tmp/appindicator-make.log 2>&1 || { echo "build failed"; tail -30 /tmp/appindicator-make.log; exit 1; }
cmake --install _b >/tmp/appindicator-install.log 2>&1 || { echo "install failed"; tail -20 /tmp/appindicator-install.log; exit 1; }

# Only the shared objects are wanted. intltool existed to configure
# libdbusmenu; the static archives and libtool files are build-time only, and
# nothing loads either of them at runtime.
rm -f /app/bin/intltool-* /app/bin/intltoolize
rm -rf /app/share/intltool /app/share/aclocal/intltool.m4
rm -f /app/lib/*.a /app/lib/*.la

echo "### tray stack installed"
ls /app/lib/ | grep -iE "ayatana|dbusmenu"
