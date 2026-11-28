#!/bin/zsh
# RIG-CHECKLIST 3.1 and 9.3 (`loop`): a long run on a timecode deck.
#
# Takes back to back into deck A for MINUTES: the pitch-step take (continuous
# play, −8 … +8 %) and the deck B take (needle drop, ±8 %, scratching). In
# the second half, loop and Echo Out presses every so often. RIG_RELOAD puts
# the track back on deck A before every take (a track runs out in minutes).
# Each take's recording is checked for dropouts; then Dub is quit and the
# script looks for stall dumps and anything left running.
#
# Before: the cable loop, Dub on the scratch library in Performance, deck A on
# TC with key lock as wanted.
#
#     RIG_RELOAD="target/rig-drive/drag 672 741 493 179" scripts/rig/soak.sh OUT_DIR 60
source "${0:A:h}/lib.sh"
OUT=${1:?out dir}; MINUTES=${2:-60}
: ${RIG_RELOAD:?set RIG_RELOAD to a command that loads the track on deck A}
EX=${RIG_ECHO_X:-697}; LY=${RIG_LOOP_Y:-431}; L4=${RIG_LOOP4_X:-553}; L8=${RIG_LOOP8_X:-585}
T=$RIG_ROOT/testdata/timecode/full
mkdir -p $OUT
echo "stalls at start $(rig_stalls), $(date +%T)" > $OUT/notes.txt
start=$(rig_now); end=$(perl -e "print $start + $MINUTES * 60")
reload() {
    if rig_locked || [ "$(rig_front)" != "Dub" ]; then echo "reload skipped $(date +%T)" >> $OUT/notes.txt; return; fi
    eval "$RIG_RELOAD"; sleep 3
}
k=0
while perl -e "exit !(time < $end - 420)"; do
    half=$(perl -e "print time > $start + $MINUTES * 30 ? 1 : 0")
    reload; rig_shot $OUT/c$k-load.png
    rig_play $OUT c$k-steps $T/sl3-serato-1122-0844-left.wav 0 324 || break
    if [ $half = 1 ]; then
        for t0 in 30 120 210; do
            rig_press $t0 loop4-in $L4 $LY; rig_press $((t0 + 7)) loop4-out $L4 $LY
            rig_press $((t0 + 20)) echo-on $EX $LY; rig_press $((t0 + 23)) echo-off $EX $LY
            rig_press $((t0 + 40)) loop8-in $L8 $LY; rig_press $((t0 + 52)) loop8-out $L8 $LY
        done
    fi
    rig_at 160; rig_shot $OUT/c$k-mid.png
    rig_wait_take >> $OUT/notes.txt
    reload
    rig_play $OUT c$k-scratch $T/sl3-deckB-60s.flac 0 64 || break
    rig_wait_take >> $OUT/notes.txt
    echo "cycle $k done $(date +%T), stalls $(rig_stalls)" >> $OUT/notes.txt
    k=$((k + 1))
done
for f in $OUT/c*-steps.wav; do "$RIG_DUB" dropout-check $f | head -12 >> $OUT/notes.txt; done
echo "stalls at end $(rig_stalls), $(date +%T)" >> $OUT/notes.txt
echo DONE >> $OUT/notes.txt
