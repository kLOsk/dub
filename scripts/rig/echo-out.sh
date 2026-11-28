#!/bin/zsh
# RIG-CHECKLIST 0.10 and 5.1 (`loop`): Echo Out on the next beat, 50 in a row.
#
# Two passes of the same 195 s of the pitch-step take into deck A from the
# track's start: untouched, then with ECHO OUT pressed (on/off 150 ms apart,
# one echo, then 49 on/off cycles). `dub echo-check` compares them.
#
# Before: the cable loop, Dub on the scratch library in Performance, deck A on
# TC. RIG_RELOAD must put the same programmed track on deck A at 0:00 — it
# runs before each pass, e.g. a `target/rig-drive/drag` from its library row.
#
#     RIG_RELOAD="target/rig-drive/drag 717 990 493 179" scripts/rig/echo-out.sh OUT_DIR BPM
source "${0:A:h}/lib.sh"
OUT=${1:?out dir}; BPM=${2:?track bpm}
: ${RIG_RELOAD:?set RIG_RELOAD to a command that loads the track on deck A at 0:00}
EX=${RIG_ECHO_X:-697}; EY=${RIG_ECHO_Y:-431}
TAKE=$RIG_ROOT/testdata/timecode/full/sl3-serato-1122-0844-left.wav

reload() { rig_locked || [ "$(rig_front)" != "Dub" ] && { echo "Dub not in front or the screen is locked"; exit 1; }; eval "$RIG_RELOAD"; sleep 3; }

reload
rig_play $OUT untouched $TAKE 0 195 || exit 1; rig_wait_take
reload
rig_play $OUT echo $TAKE 0 195 || exit 1
rig_at 12.5
if ! rig_locked && [ "$(rig_front)" = "Dub" ]; then
    at=$(perl -MTime::HiRes=time -e "printf '%.3f', time - $RIG_T0 + 0.15")
    "$RIG_DRIVE/tap2" $EX $EY 150
    print "inside-on $at\ninside-off $(perl -e "printf '%.3f', $at + 0.15")" >> $OUT/echo-presses.txt
fi
rig_press 17 single-on $EX $EY; rig_press 20 single-off $EX $EY
for k in {0..48}; do
    rig_press $((23 + 3.5 * k)) on-$k $EX $EY; rig_press $((25 + 3.5 * k)) off-$k $EX $EY
done
rig_wait_take

"$RIG_DUB" decode-timecode $TAKE --head 9000 2>/dev/null | awk '$1 ~ /^[0-9.]+$/ && NF>=3 {print $1, $2}' > $OUT/rates.txt
"$RIG_DUB" echo-check $OUT/untouched.wav $OUT/echo.wav --presses $OUT/echo-presses.txt \
    --bpm $BPM --rates $OUT/rates.txt --take-start $RIG_TAKE_START
