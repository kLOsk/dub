#!/bin/zsh
# Shared helpers for the agent's runs of docs/RIG-CHECKLIST.md (`robot` and
# `loop` items). Source it from a run script:
#
#     source scripts/rig/lib.sh
#     rig_play OUT_DIR NAME TAKE FROM SECS        # starts dub loopback, sets RIG_T0
#     rig_press 6.0 "on-0" 697 431                # click at second 6.0 of the recording
#     rig_wait_take                               # until the take has played out
#
# Every press is logged as `name seconds-of-the-recording` to
# OUT_DIR/NAME-presses.txt, the format `dub echo-check --presses` reads. A
# press is made only while Dub is the frontmost app: synthetic clicks land on
# whatever is in front, and the DJ may be using the Mac.
#
# Needs: `make rig-drive` (the click / drag / window helpers), a built
# `target/release/dub`, Accessibility and Screen Recording for the terminal.

RIG_ROOT=${RIG_ROOT:-${0:A:h:h:h}}
RIG_DRIVE=$RIG_ROOT/target/rig-drive
RIG_DUB=${RIG_DUB:-$RIG_ROOT/target/release/dub}
RIG_DEVICE=${RIG_DEVICE:-SL 3}

rig_now() { perl -MTime::HiRes=time -e 'printf "%.3f", time'; }

rig_front() {
    osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true' 2>/dev/null
}

# Locked, or a screen saver up: the login window covers the display. macOS
# still names Dub as the frontmost app then, so rig_front alone would let a
# click land on the lock screen (2026-11-27).
rig_locked() { "$RIG_DRIVE/winlist" loginwindow | awk '$2 >= 1000 {found=1} END {exit !found}'; }

# The id of Dub's main window (the large one), for screencapture -l.
rig_window() { "$RIG_DRIVE/winlist" Dub | awk '$2==0 && $7>500 {print $1; exit}'; }

# rig_shot FILE — Dub's main window, whatever is in front of it.
rig_shot() { screencapture -x -o -l"$(rig_window)" "$1"; }

# rig_play OUT_DIR NAME TAKE FROM SECS [loopback args...]
# Plays TAKE from FROM for SECS into deck A (the SL 3's aux pair, cabled
# AUX OUT → LEFT DECK IN) and records deck A's output (LEFT DECK OUT →
# AUX IN) to OUT_DIR/NAME.wav. Returns once the recording has started, with
# RIG_T0 set to its first frame (unix seconds) and RIG_TAKE_START to the
# second of the recording the take begins at.
rig_play() {
    local out=$1 name=$2 take=$3 from=$4 secs=$5; shift 5
    mkdir -p "$out"; : > "$out/$name-presses.txt"
    RIG_OUT=$out; RIG_NAME=$name; RIG_SECS=$secs
    "$RIG_DUB" loopback "$take" --device "$RIG_DEVICE" --from "$from" --secs "$secs" \
        --input-channels 1,2 --record "$out/$name.wav" "$@" > "$out/$name.log" 2>&1 &
    RIG_PID=$!
    # Keep the display awake for the take: an idle lock mid-run would turn
    # every later press into a click on the lock screen.
    caffeinate -dimsu -w "$RIG_PID" &
    local i
    for i in {1..200}; do grep -q "recording starts at unix" "$out/$name.log" && break; sleep 0.1; done
    RIG_T0=$(awk '/recording starts at unix/ {print $5}' "$out/$name.log")
    local take_unix=$(awk '/take starts at unix/ {print $5}' "$out/$name.log")
    RIG_TAKE_START=$(perl -e "printf '%.3f', $take_unix - $RIG_T0")
    [ -n "$RIG_T0" ] || { echo "loopback did not start:"; cat "$out/$name.log"; return 1; }
}

# rig_at T — sleep until second T of the recording.
rig_at() { perl -MTime::HiRes=time,sleep -e "my \$d=$1-(time-$RIG_T0); sleep(\$d) if \$d>0"; }

# rig_press T NAME X Y [HOLD_MS] — click at second T of the recording if Dub is
# in front; the logged time is the mouse-down.
rig_press() {
    local t=$1 name=$2 x=$3 y=$4 hold=${5:-60}
    rig_at "$(perl -e "printf '%.3f', $t - 0.5")"
    if rig_locked || [ "$(rig_front)" != "Dub" ]; then
        echo "$name SKIPPED" >> "$RIG_OUT/$RIG_NAME-presses.txt"; return 1
    fi
    rig_at "$(perl -e "printf '%.3f', $t - 0.15")"
    local at=$(perl -MTime::HiRes=time -e "printf '%.3f', time - $RIG_T0 + 0.15")
    "$RIG_DRIVE/click" "$x" "$y" "$hold"
    echo "$name $at" >> "$RIG_OUT/$RIG_NAME-presses.txt"
}

# rig_wait_take — until loopback has finished, then print its summary.
rig_wait_take() {
    wait "$RIG_PID" 2>/dev/null
    while kill -0 "$RIG_PID" 2>/dev/null; do sleep 0.5; done
    grep -E "recorded|peak|came back|not the take" "$RIG_OUT/$RIG_NAME.log"
}

# rig_stalls — how many stall dumps Dub has written so far.
rig_stalls() { ls ~/Library/Logs/Dub 2>/dev/null | grep -c '^stall-'; }
