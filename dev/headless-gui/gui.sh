#!/usr/bin/env bash
# Run the workbench in a headless sway and drive it over VNC: screenshots and
# input without a desktop, for checking a GUI change from a terminal or an
# agent. Run inside the gui-test shell:
#
#   nix develop .#gui-test -c dev/headless-gui/gui.sh start
#   nix develop .#gui-test -c dev/headless-gui/gui.sh app            # rusty-tip-gui
#   nix develop .#gui-test -c dev/headless-gui/gui.sh shot connection
#   nix develop .#gui-test -c dev/headless-gui/gui.sh click 400 120
#   nix develop .#gui-test -c dev/headless-gui/gui.sh stop
#
# Commands:
#   start                 headless sway plus wayvnc on 127.0.0.1:$PORT
#   app [BIN] [ARGS...]   build and launch a gui binary (default rusty-tip-gui)
#   shot NAME             screenshot to target/headless-gui/shots/NAME.png
#   click X Y | move X Y | type TEXT | key KEY | scroll X Y up|down N
#   res WxH               resize the output (checks the window's minimum size)
#   stop                  quit the app, wayvnc and sway
#
# Why sway + wayvnc: the headless seat has no pointer of its own, but wayvnc
# keeps a virtual pointer and keyboard for the whole session, so vncdo input
# lands. wlrctl's pointer dies after each call, and Xvfb loses EGL whenever
# the system Mesa is newer than the dev shell's glibc. The first keystroke
# over a fresh VNC keyboard can be dropped.
set -euo pipefail

repo=$(git rev-parse --show-toplevel)
state="$repo/target/headless-gui"
# Sockets live under the real runtime dir: a path under target/ can exceed
# the 108-byte limit on a unix socket name.
run="${XDG_RUNTIME_DIR:-/tmp}/rusty-tip-headless"
port=${HEADLESS_GUI_PORT:-5977}
mkdir -p "$state"/{shots,data,config,logs} "$run"
chmod 700 "$run"

die() { echo "gui.sh: $*" >&2; exit 1; }
alive() { [ -f "$state/$1.pid" ] && kill -0 "$(cat "$state/$1.pid")" 2>/dev/null; }
display() { cat "$state/display" 2>/dev/null || die "not started; run 'gui.sh start'"; }
sway_sock() { ls "$run"/sway-ipc.*.sock 2>/dev/null | head -1; }

# vncdotool is not in every nixpkgs pin; fall back to the registry's
# nixpkgs once and remember the store path.
vnc() {
  local bin
  if command -v vncdo >/dev/null 2>&1; then
    bin=vncdo
  else
    if [ ! -x "$(cat "$state/vncdo" 2>/dev/null)" ]; then
      nix build --no-link --print-out-paths nixpkgs#python3Packages.vncdotool \
        | sed 's|$|/bin/vncdo|' > "$state/vncdo"
    fi
    bin=$(cat "$state/vncdo")
  fi
  "$bin" -s "127.0.0.1::$port" "$@"
}

start() {
  command -v sway >/dev/null || die "sway not on PATH; run inside 'nix develop .#gui-test'"
  alive sway && { echo "already running on $(display), vnc :$port"; return; }
  printf 'output HEADLESS-1 resolution 1280x900 position 0 0\ndefault_border none\n' \
    > "$state/sway.cfg"
  (
    export XDG_RUNTIME_DIR="$run" WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
      WLR_RENDERER=pixman
    unset DISPLAY WAYLAND_DISPLAY SWAYSOCK
    setsid sway -c "$state/sway.cfg" > "$state/logs/sway.log" 2>&1 &
    echo $! > "$state/sway.pid"
  )
  local sock=""
  for _ in $(seq 50); do
    sock=$(ls "$run" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -1 || true)
    [ -n "$sock" ] && break
    sleep 0.2
  done
  [ -n "$sock" ] || die "sway did not come up; see $state/logs/sway.log"
  echo "$sock" > "$state/display"
  (
    export XDG_RUNTIME_DIR="$run" WAYLAND_DISPLAY="$sock"
    setsid wayvnc 127.0.0.1 "$port" > "$state/logs/wayvnc.log" 2>&1 &
    echo $! > "$state/wayvnc.pid"
  )
  sleep 1
  alive wayvnc || die "wayvnc did not start; see $state/logs/wayvnc.log"
  echo "sway on $sock, vnc on 127.0.0.1:$port"
}

app() {
  local bin=${1:-rusty-tip-gui}
  [ $# -gt 0 ] && shift
  local mesa=${HEADLESS_GUI_MESA:?run inside 'nix develop .#gui-test'}
  alive app && die "an app is already running; 'gui.sh stop' first"
  (cd "$repo" && cargo build -q --features gui --bin "$bin")
  (
    export XDG_RUNTIME_DIR="$run" WAYLAND_DISPLAY="$(display)" \
      XDG_DATA_HOME="$state/data" XDG_CONFIG_HOME="$state/config" \
      LD_LIBRARY_PATH="${LD_LIBRARY_PATH:+$LD_LIBRARY_PATH:}$mesa/lib" \
      __EGL_VENDOR_LIBRARY_FILENAMES="$mesa/share/glvnd/egl_vendor.d/50_mesa.json"
    unset DISPLAY
    cd "$state"
    setsid "$repo/target/debug/$bin" "$@" > "$state/logs/app.log" 2>&1 &
    echo $! > "$state/app.pid"
  )
  sleep 2
  alive app || die "$bin exited; see $state/logs/app.log"
  echo "$bin running; logs in $state/logs/app.log"
}

cmd=${1:-}
[ $# -gt 0 ] && shift
case "$cmd" in
  start) start ;;
  app) app "$@" ;;
  shot)
    [ $# -eq 1 ] || die "usage: shot NAME"
    sleep 0.6
    XDG_RUNTIME_DIR="$run" WAYLAND_DISPLAY="$(display)" grim "$state/shots/$1.png"
    echo "$state/shots/$1.png" ;;
  click) vnc move "$1" "$2" pause 0.15 click 1 ;;
  move) vnc move "$1" "$2" ;;
  type) vnc type "$1" ;;
  key) vnc key "$1" ;;
  scroll)
    button=5; [ "$3" = up ] && button=4
    vnc move "$1" "$2"
    for _ in $(seq "$4"); do vnc click "$button"; done ;;
  res) SWAYSOCK=$(sway_sock) swaymsg output HEADLESS-1 resolution "$1" ;;
  stop)
    for p in app wayvnc; do alive "$p" && kill "$(cat "$state/$p.pid")"; done
    if alive sway; then SWAYSOCK=$(sway_sock) swaymsg exit >/dev/null 2>&1 || kill "$(cat "$state/sway.pid")"; fi
    rm -f "$state"/*.pid "$state/display"
    echo stopped ;;
  *) sed -n '2,25p' "$0"; exit 2 ;;
esac
