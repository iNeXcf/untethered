#!/bin/sh
# Invoke the shared disposable-home-server runner without a route to the LAN.
set -eu
cd "$(dirname "$0")/../.."
case "${1-}" in --voice-room|--window-share|--room-camera|--room-ptt) ;; *) : "${ELEMENT_WEB_DIR:?Set to an extracted Element Web release}";; esac
: "${LIVEKIT_SERVER:?Set to the livekit-server executable}"
: "${LK_JWT_SERVICE:?Set to the lk-jwt-service executable}"
if [ "$#" -eq 0 ]; then set -- --rtc-audio; fi
exec unshare -Urn sh -ec '
    ip link set lo up
    ip link add rtc-test type dummy
    ip addr add 192.0.2.2/24 dev rtc-test
    ip link set rtc-test up
    exec "${PYTHON:-python3}" scripts/test-media-interop.py "$@"
' rtc "$@"
