// Build with C++17, the repository include path, and pkg-config --cflags --libs Qt6Core.
#include "matrixrtcwire.h"

#include <cstdlib>
#include <iostream>
#include <stdexcept>

namespace {
void require(bool condition, const char *message) {
    if (!condition) throw std::runtime_error(message);
}
}

int main() {
    using namespace MatrixRtcWire;
    try {
        const Peer sender{"@alex:example.org", "DEVICE-A", "membership-a"};
        const Peer target{"@mira:example.org", "DEVICE-B", "membership-b"};
        const QString room = "!test:example.org";
        const MediaKey key{QByteArray(16, 'k'), 3};

        require(livekitIdentity(sender) == "i2p8bmQm4rqwCC2xcUo58Z6ozBaYwz4ewYVXIF2+wlo", "Wrong MatrixRTC identity");

        const auto token = tokenRequest(QUrl("wss://rtc.example.org"), room, "m.call#room", sender, "example.org");
        require(token && (*token)["member_id"].toString() == sender.memberId
                    && (*token)["server_name"].toString() == "example.org", "Invalid token request");
        require(!tokenRequest(QUrl("ws://rtc.example.org"), room, "m.call#room", sender), "Accepted insecure transport");

        const auto request = encryptedKeyRequest(room, sender, target, key);
        require(request && (*request)["encrypted"].toBool(), "Missing gomuks encryption flag");
        const auto content = (*request)["messages"].toObject().value(target.userId).toObject().value(target.deviceId);
        const QJsonObject event{{"sender", sender.userId}, {"encrypted", true}, {"type", KeyEventType}, {"content", content}};
        const auto decoded = receivedKey(event, room, sender);
        require(decoded && decoded->bytes == key.bytes && decoded->index == key.index, "Key failed to round-trip");
        require(!receivedKey(event, "!other:example.org", sender), "Accepted a key for another room");
        require(!receivedKey(event, room, {sender.userId, "OTHER-DEVICE", sender.memberId}), "Accepted a key for another device");
        require(!receivedKey(event, room, {sender.userId, sender.deviceId, "old-member"}), "Accepted a stale membership key");
        auto clearEvent = event;
        clearEvent.insert("encrypted", false);
        require(!receivedKey(clearEvent, room, sender), "Accepted a plaintext key");
        require(!encryptedKeyRequest(room, sender, sender, key), "Addressed a key to its own sender device");
        require(isJoinedMember(QJsonObject{}) && !isJoinedMember(QJsonObject{{"membership", "leave"}}),
                "Incorrect legacy join or explicit leave handling");
        std::cout << "MatrixRTC identity, token and encrypted key envelopes passed.\n";
        return EXIT_SUCCESS;
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return EXIT_FAILURE;
    }
}
