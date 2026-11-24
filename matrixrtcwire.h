#ifndef MATRIXRTCWIRE_H
#define MATRIXRTCWIRE_H

#include <QByteArray>
#include <QCryptographicHash>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QString>
#include <QUrl>
#include <optional>

namespace MatrixRtcWire {

inline constexpr auto KeyEventType = "io.element.call.encryption_keys";

struct Peer {
    QString userId;
    QString deviceId;
    QString memberId;
};

struct MediaKey {
    QByteArray bytes;
    int index;
};

inline bool isRoomCallSlot(const QString &slot) {
    return slot == "m.call#room" || slot == "m.call#ROOM";
}

inline bool isJoinedMember(const QJsonObject &member) {
    // Earlier Element versions omit membership on join events.
    return !member.contains("membership") || member["membership"].toString() == "join";
}

inline QByteArray livekitIdentity(const Peer &peer) {
    const auto canonical = QJsonDocument(QJsonArray{peer.userId, peer.memberId}).toJson(QJsonDocument::Compact);
    return QCryptographicHash::hash(canonical, QCryptographicHash::Sha256)
        .toBase64(QByteArray::OmitTrailingEquals);
}

inline std::optional<QJsonObject> tokenRequest(const QUrl &transport, const QString &roomId,
                                               const QString &slotId, const Peer &self,
                                               const QString &serverName = {}) {
    if (!transport.isValid() || transport.scheme() != "wss" || transport.host().isEmpty()
        || !transport.userInfo().isEmpty() || transport.hasFragment() || !roomId.startsWith('!')
        || !isRoomCallSlot(slotId) || self.memberId.isEmpty()) return std::nullopt;

    QJsonObject request{{"url", transport.toString(QUrl::FullyEncoded)}, {"room_id", roomId},
                        {"slot_id", slotId}, {"member_id", self.memberId}};
    if (!serverName.isEmpty()) request.insert("server_name", serverName);
    return request;
}

inline bool isMediaKey(const MediaKey &key) {
    return (key.bytes.size() == 16 || key.bytes.size() == 32) && key.index >= 0 && key.index <= 255;
}

inline std::optional<QJsonObject> encryptedKeyRequest(const QString &roomId, const Peer &self,
                                                      const Peer &target, const MediaKey &key) {
    if (!isMediaKey(key) || !roomId.startsWith('!') || self.memberId.isEmpty() || self.deviceId.isEmpty()
        || !target.userId.startsWith('@') || target.deviceId.isEmpty()
        || (self.userId == target.userId && self.deviceId == target.deviceId)) return std::nullopt;

    const QJsonObject mediaKey{{"key", QString::fromLatin1(key.bytes.toBase64(QByteArray::OmitTrailingEquals))},
                               {"index", key.index}};
    const QJsonObject member{{"id", self.memberId}, {"claimed_device_id", self.deviceId}};
    const QJsonObject content{{"room_id", roomId}, {"member_id", self.memberId}, {"member", member},
                              {"media_key", mediaKey}, {"keys", mediaKey}};
    const QJsonObject devices{{target.deviceId, content}};
    const QJsonObject messages{{target.userId, devices}};
    return QJsonObject{{"event_type", KeyEventType}, {"encrypted", true}, {"messages", messages}};
}

inline std::optional<MediaKey> receivedKey(const QJsonObject &event, const QString &roomId, const Peer &activePeer) {
    if (!event["encrypted"].toBool() || event["type"].toString() != KeyEventType
        || activePeer.userId.isEmpty() || activePeer.deviceId.isEmpty() || activePeer.memberId.isEmpty()
        || event["sender"].toString() != activePeer.userId) return std::nullopt;

    const auto content = event["content"].toObject();
    const auto member = content["member"].toObject();
    const auto memberId = content["member_id"].toString(member["id"].toString());
    if (content["room_id"].toString() != roomId || member["claimed_device_id"].toString() != activePeer.deviceId
        || memberId != activePeer.memberId) return std::nullopt;

    for (const auto *field : {"media_key", "keys"}) {
        const auto mediaKey = content[field].toObject();
        const auto encoded = mediaKey["key"].toString().toLatin1();
        const auto decoded = QByteArray::fromBase64Encoding(encoded, QByteArray::AbortOnBase64DecodingErrors);
        if (!decoded) continue;
        const MediaKey key{decoded.decoded, mediaKey["index"].toInt(-1)};
        if (isMediaKey(key)) return key;
    }
    return std::nullopt;
}

} // namespace MatrixRtcWire

#endif
