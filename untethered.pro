QT += quick network websockets multimedia concurrent
CONFIG += c++17
TARGET = untethered
SOURCES += main.cpp
HEADERS += matrixrtcwire.h
RESOURCES += ui.qrc

LIVEKIT_SDK = $$(LIVEKIT_SDK)
isEmpty(LIVEKIT_SDK): error("Set LIVEKIT_SDK to the unpacked LiveKit C++ SDK.")
LIVEKIT_RUNPATH = $$(LIVEKIT_RUNPATH)
isEmpty(LIVEKIT_RUNPATH): LIVEKIT_RUNPATH = $$LIVEKIT_SDK/lib
INCLUDEPATH += $$LIVEKIT_SDK/include
LIBS += -L$$LIVEKIT_SDK/lib -llivekit
QMAKE_RPATHDIR += $$LIVEKIT_RUNPATH
