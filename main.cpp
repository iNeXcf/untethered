#include "matrixrtcwire.h"

#include <QtCore>
#include <QGuiApplication>
#include <QClipboard>
#include <QImageReader>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlNetworkAccessManagerFactory>
#include <QtNetwork>
#include <QWebSocket>
#include <functional>
#include <memory>
#include <utility>
#include <algorithm>
#include <QtConcurrent>
#include <QCamera>
#include <QMediaCaptureSession>
#include <QMediaDevices>
#include <QScreenCapture>
#include <QVideoSink>
#include <QWindowCapture>
#include <mutex>
#include <thread>
#include <livekit/livekit.h>

class MediaNetworkFactory : public QQmlNetworkAccessManagerFactory {
public:
    QNetworkAccessManager *create(QObject *parent) override {
        auto *manager = new QNetworkAccessManager(parent);
        manager->setProxy(QNetworkProxy::NoProxy);
        manager->setRedirectPolicy(QNetworkRequest::ManualRedirectPolicy);
        manager->setTransferTimeout(15000);
        return manager;
    }
};

class Chat : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool ready MEMBER ready NOTIFY changed)
    Q_PROPERTY(bool loggedIn MEMBER loggedIn NOTIFY changed)
    Q_PROPERTY(bool loggingOut READ loggingOut NOTIFY changed)
    Q_PROPERTY(bool nativeCall READ nativeCall NOTIFY callControlsChanged)
    Q_PROPERTY(bool nativeConnected MEMBER nativeConnected NOTIFY callControlsChanged)
    Q_PROPERTY(bool nativeCamera READ nativeCamera NOTIFY callControlsChanged)
    Q_PROPERTY(bool nativeSharing READ nativeSharing NOTIFY callControlsChanged)
    Q_PROPERTY(QVariantList nativeVideoTiles MEMBER nativeVideoTiles NOTIFY nativeVideoTilesChanged)
    Q_PROPERTY(bool nativeMuted MEMBER nativeMuted NOTIFY callControlsChanged)
    Q_PROPERTY(QVariantList nativeInputs READ nativeInputs NOTIFY callControlsChanged)
    Q_PROPERTY(QVariantList nativeOutputs READ nativeOutputs NOTIFY callControlsChanged)
    Q_PROPERTY(QString callRoomId READ activeCallRoomId NOTIFY callControlsChanged)
    Q_PROPERTY(QString callRoomName MEMBER callRoomName NOTIFY callControlsChanged)
    Q_PROPERTY(QString callTransportStatus MEMBER callTransportStatus NOTIFY callControlsChanged)
    Q_PROPERTY(QVariantList callParticipants MEMBER callParticipants NOTIFY callParticipantsChanged)
    Q_PROPERTY(QString imageUploadRoom MEMBER imageUploadRoom NOTIFY changed)
    Q_PROPERTY(bool verified MEMBER verified NOTIFY changed)
    Q_PROPERTY(QString status MEMBER status NOTIFY changed)
    Q_PROPERTY(QString profile MEMBER profile NOTIFY changed)
    Q_PROPERTY(QString deviceId MEMBER deviceId NOTIFY changed)
    Q_PROPERTY(QString profileName READ profileName NOTIFY changed)
    Q_PROPERTY(QUrl profileAvatar READ profileAvatar NOTIFY changed)
    Q_PROPERTY(QString selected MEMBER selected NOTIFY changed)
    Q_PROPERTY(QString roomName MEMBER roomName NOTIFY changed)
    Q_PROPERTY(bool encrypted MEMBER encrypted NOTIFY changed)
    Q_PROPERTY(QVariantList roomList READ roomList NOTIFY changed)
    Q_PROPERTY(QVariantList messages READ messages NOTIFY timelineChanged)

public:
    Chat(QString executable, QString root) {
        livekit::initialize(livekit::LogLevel::Error);
        nativeVideoTimer.setInterval(33);
        connect(&nativeVideoTimer, &QTimer::timeout, this, &Chat::renderNativeVideo);
        nativeMediaTimer.setInterval(500);
        connect(&nativeMediaTimer, &QTimer::timeout, this, &Chat::refreshNativeMedia);
        for (int i = 0; i < 8; ++i)
            key += QByteArray::number(QRandomGenerator::system()->generate(), 16).rightJustified(8, '0');
        http.setProxy(QNetworkProxy::NoProxy);
        socket.setProxy(QNetworkProxy::NoProxy);
        callMembersTimer.setInterval(6000);
        connect(&callMembersTimer, &QTimer::timeout, this, &Chat::refreshCallParticipants);
        connect(&process, &QProcess::readyReadStandardOutput, this, &Chat::readStartup);
        connect(&process, &QProcess::readyReadStandardError, this, [this] { process.readAllStandardError(); });
        connect(&process, &QProcess::errorOccurred, this, [this](auto) { setStatus("Could not start the local gomuks backend."); });
        connect(&process, &QProcess::finished, this, [this] {
            startup.stop();
            ready = false;
            setStatus("The local backend stopped. Reopen the app to restart it.");
        });
        connect(&socket, &QWebSocket::textMessageReceived, this, &Chat::receive);
        connect(&socket, &QWebSocket::connected, this, [this] {
            startup.stop();
            misses = 0;
            ping.start(15000);
            setStatus("Loading your Matrix session…");
        });
        connect(&socket, &QWebSocket::disconnected, this, [this] {
            ping.stop();
            ready = false;
            if (nativeCall()) closeCall();
            pending.clear();
            callMemberRequest = 0;
            if (callTransportRequest != 0)
                finishCallTransportDiscovery(tr("Could not check the homeserver’s MatrixRTC transport."));
            logoutRequest = 0;
            cancelImageUpload();
            setStatus("Disconnected. Reconnecting…");
            QTimer::singleShot(1200, this, [this] {
                if (process.state() == QProcess::Running) authenticate();
            });
        });
        connect(&socket, &QWebSocket::errorOccurred, this, [this](auto) {
            startup.stop();
            ready = false;
            setStatus("Could not connect to the local backend.");
        });
        connect(&ping, &QTimer::timeout, this, [this] {
            if (++misses > 3) {
                socket.abort();
                return;
            }
            transmit("ping", {{"last_received_id", lastEvent}});
        });
        startup.setSingleShot(true);
        connect(&startup, &QTimer::timeout, this, [this] { setStatus("The local backend did not connect in time."); });
        auto env = QProcessEnvironment::systemEnvironment();
        for (const auto &name : env.keys())
            if (name.startsWith("GOMUKS_")) env.remove(name);
        env.insert("GOMUKS_ROOT", root);
        env.insert("GOMUKS_DESKTOP_KEY", QString::fromLatin1(key));
        process.setProcessEnvironment(env);
        if (!executable.isEmpty()) {
            process.start(executable, {"--desktop"});
            startup.start(15000);
        } else {
            status = "No bundled backend found.";
        }
    }

    ~Chat() override {
        closeCall();
        if (nativeJoin.isValid()) nativeJoin.waitForFinished();
        delete nativeJoinWatcher.data();
        nativeJoin = {};
        livekit::shutdown();
        socket.blockSignals(true);
        process.blockSignals(true);
        socket.abort();
        cancelImageUpload();
        if (process.state() != QProcess::NotRunning) {
            process.terminate();
            if (!process.waitForFinished(3500)) {
                process.kill();
                process.waitForFinished(1000);
            }
        }
    }

    Q_INVOKABLE void login(QString homeserver, QString user, QString password) {
        const auto url = QUrl(homeserver.trimmed());
        if (url.scheme() != "https" || url.host().isEmpty() || !url.userInfo().isEmpty() ||
            user.trimmed().isEmpty() || password.isEmpty()) {
            setStatus("Enter an HTTPS homeserver, username and password.");
            return;
        }
        rpc("login", {{"homeserver_url", url.toString()}, {"username", user.trimmed()}, {"password", password}});
    }
    bool loggingOut() const { return logoutRequest != 0; }

    Q_INVOKABLE void logout() {
        if (!ready || !loggedIn || loggingOut()) return;
        closeCall();
        cancelImageUpload();
        logoutRequest = transmit("logout", {});
        setStatus("Signing out…");
    }

    Q_INVOKABLE void verify(QString key) {
        if (!key.trimmed().isEmpty()) rpc("verify", {{"recovery_key", key.trimmed()}});
    }
    Q_INVOKABLE void openRoom(QString room) {
        if (selected == room || (!room.isEmpty() && !rooms.contains(room))) return;
        selected = room;
        selectionChanged();
        loadRoomProfiles();
        paginate();
    }
    Q_INVOKABLE void paginate(bool older = false) {
        if (selected.isEmpty() || !ready) return;
        const auto room = selected;
        double oldest = 0;
        for (const auto &event : history[room]) {
            double row = event["timeline_rowid"].toDouble();
            if (row != 0 && (oldest == 0 || row < oldest)) oldest = row;
        }
        rpc("paginate", {{"room_id", room}, {"limit", 40}, {"max_timeline_id", oldest}}, [this, room, older](QJsonValue reply) {
            const auto events = reply.toObject()["events"].toArray();
            if (events.isEmpty()) return;
            if (older && room == selected) emit olderMessagesLoading();
            for (const auto &event : events) insert(room, event.toObject());
            if (room == selected) emit timelineChanged();
        });
    }
    QString activeCallRoomId() const { return callRoomId; }

    bool nativeCall() const { return !callRoomId.isEmpty(); }

    QVariantList nativeInputs() const { return nativeDevices(true); }
    QVariantList nativeOutputs() const { return nativeDevices(false); }

    bool nativeCamera() const {
        return localCamera != nullptr;
    }

    bool nativeSharing() const {
        return localShare != nullptr;
    }

    Q_INVOKABLE void setNativeVideoSink(const QString &id, QObject *sink) {
        if (auto *video = qobject_cast<QVideoSink *>(sink)) nativeSinks[id] = video;
    }

    Q_INVOKABLE QVariantMap nativeShareSources() {
        shareScreens.clear();
        shareWindows.clear();
        QVariantList screens, windows;
        for (auto *screen : QGuiApplication::screens()) {
            shareScreens.append(screen);
            screens.append(screen->name().isEmpty() ? tr("Screen %1").arg(screens.size() + 1) : screen->name());
        }
        if (!QGuiApplication::platformName().startsWith("wayland")) {
            shareWindows = QWindowCapture::capturableWindows();
            for (const auto &window : std::as_const(shareWindows)) windows.append(window.description());
        }
        return {{"screens", screens}, {"windows", windows},
                {"portal", QGuiApplication::platformName().startsWith("wayland")}};
    }

    Q_INVOKABLE void startNativeShare(int index, bool window) {
        if (localShare) return;
        if (window) {
            const auto source = shareWindows.value(index);
            if (!source.isValid()) return;
            toggleNativeVideo(true, nullptr, source);
        } else {
            auto *screen = index < 0 ? QGuiApplication::primaryScreen() : shareScreens.value(index).data();
            if (screen) toggleNativeVideo(true, screen);
        }
    }

    Q_INVOKABLE void toggleNativeCamera() { toggleNativeVideo(false); }
    Q_INVOKABLE void stopNativeShare() {
        if (localShare) toggleNativeVideo(true);
    }

    void toggleNativeVideo(bool share, QScreen *screen = nullptr, const QCapturableWindow &window = {}) {
        if (!nativeConnected || !nativeSession) return;
        auto &capture = share ? localShare : localCamera;
        if (capture) {
            stopNativeCapture(capture);
            emit callControlsChanged();
            return;
        }
        capture = std::make_unique<NativeCapture>();
        capture->session.setVideoSink(&capture->sink);
        connect(&capture->sink, &QVideoSink::videoFrameChanged, this, [this, share](const QVideoFrame &frame) {
            pushNativeVideo(share, frame);
        });
        if (share && window.isValid()) {
            capture->window = std::make_unique<QWindowCapture>();
            capture->window->setWindow(window);
            capture->session.setWindowCapture(capture->window.get());
            connect(capture->window.get(), &QWindowCapture::errorOccurred, this, [this, owner = &capture->session] {
                deferNativeCaptureFailure(true, owner);
            });
            capture->window->start();
        } else if (share) {
            capture->screen = std::make_unique<QScreenCapture>();
            capture->screen->setScreen(screen);
            capture->session.setScreenCapture(capture->screen.get());
            connect(capture->screen.get(), &QScreenCapture::errorOccurred, this, [this, owner = &capture->session] {
                deferNativeCaptureFailure(true, owner);
            });
            capture->screen->start();
        } else {
            const auto device = QMediaDevices::defaultVideoInput();
            if (device.isNull()) {
                failNativeCapture(false);
                return;
            }
            capture->camera = std::make_unique<QCamera>(device);
            capture->session.setCamera(capture->camera.get());
            connect(capture->camera.get(), &QCamera::errorOccurred, this, [this, owner = &capture->session] {
                deferNativeCaptureFailure(false, owner);
            });
            capture->camera->start();
        }
        emit callControlsChanged();
    }

    Q_INVOKABLE void setNativeDevice(bool input, const QString &id) {
        if (!nativeSession || id.isEmpty()) return;
        try {
            if (input) nativeSession->audio.setRecordingDevice(id.toStdString());
            else nativeSession->audio.setPlayoutDevice(id.toStdString());
        } catch (const std::exception &) {
            finishCallTransportDiscovery(tr("Could not select the audio device."));
        }
    }

    Q_INVOKABLE void setNativeOutputMuted(bool muted) {
        nativeOutputMuted = muted;
        syncNativeSubscriptions();
    }

    Q_INVOKABLE void toggleNativeMicrophone() {
        if (!nativeSession || !nativeConnected) return;
        try {
            if (!nativeSession->microphone) {
                const auto participant = nativeSession->room.localParticipant().lock();
                if (!participant) return;
                auto source = nativeSession->audio.createAudioSource();
                auto track = livekit::LocalAudioTrack::createLocalAudioTrack("microphone", source);
                livekit::TrackPublishOptions options;
                options.source = livekit::TrackSource::SOURCE_MICROPHONE;
                participant->publishTrack(track, options);
                nativeSession->microphone = std::move(track);
            } else if (nativeMuted) {
                nativeSession->microphone->unmute();
            } else {
                nativeSession->microphone->mute();
            }
            nativeMuted = !nativeMuted;
            emit callControlsChanged();
        } catch (const std::exception &) {
            finishCallTransportDiscovery(tr("Could not change the microphone state."));
        }
    }

    Q_INVOKABLE void openCall() {
        if (!ready || !loggedIn || loggingOut() || selected.isEmpty() || base.isEmpty() || nativeCall()) return;
        callRoomName = roomName;
        callRoomId = selected;
        nativeSelf = {profile, deviceId, QUuid::createUuid().toString(QUuid::WithoutBraces)};
        discoverCallTransports();
        refreshCallParticipants();
        callMembersTimer.start();
    }

    Q_INVOKABLE void closeCall() {
        if (callRoomId.isEmpty()) return;
        closeNativeCall();
        callRoomName.clear();
        callRoomId.clear();
        callMembersTimer.stop();
        callMemberRequest = 0;
        callStickyMembers = {};
        callParticipants.clear();
        emit callParticipantsChanged();
        callTransports.clear();
        callTransportRequest = 0;
        callTransportStatus.clear();
        emit callControlsChanged();
    }

    Q_INVOKABLE void copyText(const QString &text) {
        QGuiApplication::clipboard()->setText(text);
    }
    Q_INVOKABLE bool canAttachImage(const QUrl &url) const {
        return url.isLocalFile() && QImageReader(url.toLocalFile()).canRead();
    }

    Q_INVOKABLE QUrl clipboardImage() const {
        const auto image = QGuiApplication::clipboard()->image();
        if (image.isNull()) return {};
        QByteArray bytes;
        QBuffer buffer(&bytes);
        if (!buffer.open(QIODevice::WriteOnly) || !image.save(&buffer, "PNG")) return {};
        return QUrl("data:image/png;base64," + QString::fromLatin1(bytes.toBase64()));
    }

    Q_INVOKABLE bool send(QString text, QString replyTo) {
        if (!ready || loggingOut() || selected.isEmpty() || text.trimmed().isEmpty()) return false;
        if (!replyTo.isEmpty() && !replyTo.startsWith('$')) return false;
        const auto room = selected;
        rpc("send_message", messageParams(room, text, replyTo), [this, room](QJsonValue reply) {
            insert(room, reply.toObject());
            if (room == selected) emit timelineChanged();
        });
        return true;
    }

    Q_INVOKABLE void sendImage(const QUrl &source, const QString &caption, const QString &replyTo) {
        if (!ready || !loggedIn || loggingOut() || selected.isEmpty() || !imageUploadRoom.isEmpty()) return;
        if (!replyTo.isEmpty() && !replyTo.startsWith('$')) return;

        std::unique_ptr<QIODevice> input;
        QString filename;
        if (source.isLocalFile()) {
            const QString path = source.toLocalFile();
            auto file = std::make_unique<QFile>(path);
            filename = QFileInfo(path).fileName();
            input = std::move(file);
        } else {
            const QString prefix = "data:image/png;base64,";
            const QString data = source.toString(QUrl::FullyEncoded);
            if (data.startsWith(prefix)) {
                auto buffer = std::make_unique<QBuffer>();
                buffer->setData(QByteArray::fromBase64(data.sliced(prefix.size()).toLatin1()));
                filename = "clipboard.png";
                input = std::move(buffer);
            }
        }
        if (!input || !input->open(QIODevice::ReadOnly) || QImageReader::imageFormat(input.get()).isEmpty()) {
            setStatus("Could not read the selected image.");
            return;
        }
        input->seek(0);

        const auto room = selected;
        const bool encrypt = encrypted;
        QUrl url = base;
        url.setPath("/_gomuks/upload");
        url.setQuery(QString("encrypt=%1&filename=%2").arg(encrypt ? "true" : "false",
                         QString::fromLatin1(QUrl::toPercentEncoding(filename))));
        QNetworkRequest request(url);
        request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::ManualRedirectPolicy);
        request.setTransferTimeout(120000);
        request.setHeader(QNetworkRequest::ContentTypeHeader, "application/octet-stream");
        request.setHeader(QNetworkRequest::ContentLengthHeader, input->size());
        auto *reply = http.post(request, input.get());
        input.release()->setParent(reply);
        imageUpload = reply;
        imageUploadRoom = room;
        setStatus("");

        connect(reply, &QNetworkReply::finished, this, [this, reply, room, encrypt, caption, replyTo] {
            reply->deleteLater();
            if (imageUpload != reply) return;
            imageUpload = nullptr;
            const int code = reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
            auto content = QJsonDocument::fromJson(reply->readAll()).object();
            const auto mxc = encrypt ? content["file"].toObject()["url"].toString() : content["url"].toString();
            if (reply->error() != QNetworkReply::NoError || code < 200 || code >= 300
                || content["msgtype"].toString() != "m.image" || !mxc.startsWith("mxc://")) {
                cancelImageUpload();
                setStatus("Could not upload the image. You can try again.");
                return;
            }
            if (!ready || !rooms.contains(room) || rooms.value(room)["encryption_event"].isObject() != encrypt) {
                cancelImageUpload();
                setStatus("The room changed during upload. The image was not sent.");
                return;
            }

            // Keep captions literal so a leading slash cannot turn an image into a command.
            if (!caption.trimmed().isEmpty()) content.insert("body", caption);
            auto params = messageParams(room, "", replyTo);
            params.insert("base_content", content);
            imageSendRequest = transmit("send_message", params);
        });
    }

    QString profileName() const { return accountName.isEmpty() ? profile : accountName; }
    QUrl profileAvatar() const { return avatarUrl(accountAvatar); }

    QVariantList roomList() const {
        QVariantList values;
        for (auto it = rooms.begin(); it != rooms.end(); ++it) {
            const auto meta = it.value();
            const auto name = meta["name"].toString(it.key());
            const bool space = meta["creation_content"].toObject()["type"].toString() == "m.space" || spaces.contains(it.key());
            values.append(QVariantMap{{"id", it.key()}, {"name", name}, {"space", space},
                                      {"avatar", avatarUrl(meta["avatar"].toString())},
                                      {"dm", !meta["dm_user_id"].toString().isEmpty()},
                                      {"unread", meta["marked_unread"].toBool() || meta["unread_messages"].toInt() > 0}});
        }
        std::sort(values.begin(), values.end(), [](const QVariant &a, const QVariant &b) {
            return a.toMap()["name"].toString().compare(b.toMap()["name"].toString(), Qt::CaseInsensitive) < 0;
        });
        return values;
    }
    Q_INVOKABLE bool inSpace(QString space, QString room) const {
        if (space.isEmpty()) return true;
        for (const auto &edge : spaces.value(space).toArray()) {
            if (edge.toObject()["child_id"].toString() == room) return true;
        }
        return false;
    }
    QVariantList messages() const {
        auto list = history.value(selected).values();
        std::sort(list.begin(), list.end(), [](const QJsonObject &a, const QJsonObject &b) {
            const auto timeA = a["timestamp"].toDouble(), timeB = b["timestamp"].toDouble();
            return timeA == timeB ? a["rowid"].toDouble() < b["rowid"].toDouble() : timeA < timeB;
        });
        QVariantList result;
        const auto &profiles = roomProfiles.value(selected);
        QString previousSender;
        QDateTime previousStamp;
        constexpr qint64 messageGroupSeconds = 5 * 60;
        const QLocale locale;
        for (const auto &event : list) {
            const auto type = event["type"].toString();
            const auto content = event["content"].toObject();
            const auto body = type == "m.room.encrypted" ? event["decrypted"].toObject() : content;
            const auto stamp = QDateTime::fromMSecsSinceEpoch(qint64(event["timestamp"].toDouble()));
            QString text = body["body"].toString(type == "m.room.encrypted" ? "Waiting for encryption keys…" : "[Attachment or non-text message]");
            const auto eventId = event["event_id"].toString();
            const bool redacted = !event["redacted_by"].toString().isEmpty();
            QString replyTo;
            if (!redacted) {
                const auto replyEventId = [](const QJsonObject &message) {
                    return message["m.relates_to"].toObject()["m.in_reply_to"].toObject()["event_id"].toString();
                };
                replyTo = replyEventId(body);
                if (replyTo.isEmpty() && type == "m.room.encrypted")
                    replyTo = replyEventId(content);
            }
            if (!replyTo.isEmpty()) {
                static const QRegularExpression fallback(R"(\A> (?:\* )?<@[^>\r\n]+>[^\r\n]*\r?\n(?:>[^\r\n]*\r?\n)*\r?\n)");
                text.remove(fallback);
            }
            const bool hasImage = !redacted && body["msgtype"].toString() == "m.image";
            QUrl image, thumbnail;
            const auto imageInfo = body["info"].toObject();
            if (hasImage) {
                image = messageMediaUrl(body, "url", "file");
                thumbnail = messageMediaUrl(imageInfo, "thumbnail_url", "thumbnail_file");
            }
            if (redacted) text = "Message removed";
            else if (!event["send_error"].toString().isEmpty()) text += "  ·  Failed to send";
            const auto sender = event["sender"].toString();
            const bool startsDay = !previousStamp.isValid() || stamp.date() != previousStamp.date();
            const bool startsGroup = startsDay || !replyTo.isEmpty() || sender != previousSender
                || previousStamp.secsTo(stamp) > messageGroupSeconds;
            const auto localpart = sender.startsWith('@') ? sender.sliced(1).section(':', 0, 0) : sender;
            const auto member = profiles.value(sender);
            QString name = member["displayname"].toString();
            QString avatar = member["avatar_url"].toString();
            if (sender == profile && member.isEmpty()) {
                name = accountName;
                avatar = accountAvatar;
            }
            if (name.isEmpty()) name = localpart.isEmpty() ? tr("Unknown sender") : localpart;
            const auto encryptedAvatar = member["avatar_file"].toObject()["url"].toString();
            result.append(QVariantMap{{"id", QString::number(event["rowid"].toInteger())},
                                      {"eventId", eventId}, {"replyTo", replyTo},
                                      {"hasImage", hasImage}, {"image", image}, {"thumbnail", thumbnail},
                                      {"imageWidth", imageInfo["w"].toInt()}, {"imageHeight", imageInfo["h"].toInt()},
                                      {"canReply", !redacted && eventId.startsWith('$')},
                                      {"sender", sender}, {"name", name},
                                      {"avatar", encryptedAvatar.isEmpty() ? avatarUrl(avatar) : avatarUrl(encryptedAvatar, true)},
                                      {"startsGroup", startsGroup},
                                      {"day", startsDay ? locale.toString(stamp.date(), QLocale::LongFormat) : QString()},
                                      {"time", locale.toString(stamp.time(), QLocale::ShortFormat)},
                                      {"body", text}});
            previousSender = sender;
            previousStamp = stamp;
        }
        return result;
    }

signals:
    void changed();
    void timelineChanged();
    void olderMessagesLoading();
    void imageSent(const QString &room);
    void callControlsChanged();
    void nativeVideoTilesChanged();
    void callParticipantsChanged();

private:
    void requestCallMembers(const QString &command) {
        const int request = callMemberRequest = transmit(command, {{"room_id", callRoomId}});
        QTimer::singleShot(5000, this, [this, request] {
            if (callMemberRequest == request) callMemberRequest = 0;
        });
    }

    void refreshCallParticipants() {
        if (!ready || loggingOut() || callRoomId.isEmpty() || callMemberRequest != 0) return;
        callStickyMembers = {};
        waitingForCallState = false;
        requestCallMembers("get_sticky_events");
    }

    void readCallParticipants(const QJsonArray &legacyState) {
        const auto now = QDateTime::currentMSecsSinceEpoch();
        QMap<QString, QMap<QString, QJsonObject>> latest;
        for (const auto &entry : callStickyMembers) {
            const auto event = entry.toObject();
            const auto type = event["type"].toString();
            if ((type != "m.rtc.member" && type != "org.matrix.msc4143.rtc.member")
                || !event["redacted_by"].toString().isEmpty()) continue;
            const auto sender = event["sender"].toString();
            const auto content = event["content"].toObject();
            const auto id = content["member"].toObject()["id"].toString();
            const auto slot = content["slot_id"].toString();
            if (sender.isEmpty() || id.isEmpty() || !MatrixRtcWire::isRoomCallSlot(slot)) continue;
            auto &current = latest[sender][id];
            const auto stamp = event["timestamp"].toInteger();
            if (current.isEmpty() || stamp > current["timestamp"].toInteger()
                || (stamp == current["timestamp"].toInteger()
                    && event["event_id"].toString() > current["event_id"].toString()))
                current = event;
        }

        QSet<QString> participants;
        const auto previousNativePeers = nativePeers;
        if (nativeCall()) nativePeers.clear();
        for (auto user = latest.cbegin(); user != latest.cend(); ++user) {
            for (const auto &event : user.value()) {
                const auto member = event["content"].toObject()["member"].toObject();
                const auto duration = event["sticky_duration_ms"].toInteger();
                const auto timestamp = event["timestamp"].toInteger();
                if (MatrixRtcWire::isJoinedMember(member) && duration > 0 && timestamp > 0
                    && duration > now - timestamp) {
                    participants.insert(user.key());
                    const auto device = member["device_id"].toString();
                    const auto matrixMembership = roomProfiles.value(callRoomId).value(user.key())["membership"].toString();
                    if (nativeCall() && !device.isEmpty() && (matrixMembership.isEmpty() || matrixMembership == "join")
                        && (user.key() != profile || device != deviceId))
                        nativePeers.append({user.key(), device, member["id"].toString()});
                }
            }
        }

        for (const auto &entry : legacyState) {
            const auto event = entry.toObject();
            const auto type = event["type"].toString();
            if ((type != "org.matrix.msc3401.call.member" && type != "m.call.member")
                || !event["redacted_by"].toString().isEmpty()) continue;
            const auto sender = event["sender"].toString();
            const auto content = event["content"].toObject();
            const auto duration = content["expires"].toInteger(4 * 60 * 60 * 1000);
            const auto created = content["created_ts"].toInteger(event["timestamp"].toInteger());
            if (!sender.isEmpty() && content["application"].toString() == "m.call"
                && content["call_id"].toString().isEmpty() && !content["device_id"].toString().isEmpty()
                && duration > 0 && created > 0 && duration > now - created)
            {
                participants.insert(sender);
                const auto device = content["device_id"].toString();
                const auto memberId = content["membershipID"].toString(sender + ':' + device);
                const auto matrixMembership = roomProfiles.value(callRoomId).value(sender)["membership"].toString();
                const bool hasStickyDevice = std::any_of(nativePeers.cbegin(), nativePeers.cend(),
                    [&](const auto &peer) { return peer.userId == sender && peer.deviceId == device; });
                if (!hasStickyDevice && (sender != profile || device != deviceId)
                    && (matrixMembership.isEmpty() || matrixMembership == "join"))
                    nativePeers.append({sender, device, memberId});
            }
        }

        if (nativeCall()) {
            bool departed = false;
            for (const auto &peer : previousNativePeers) {
                const auto id = MatrixRtcWire::livekitIdentity(peer);
                const bool stillPresent = std::any_of(nativePeers.cbegin(), nativePeers.cend(),
                    [&id](const auto &entry) { return MatrixRtcWire::livekitIdentity(entry) == id; });
                if (!stillPresent && (nativeKeyRecipients.contains(id) || nativeKeyRequests.contains(id))) departed = true;
            }
            if (departed && nativeConnected && !rotateNativeKey()) {
                closeNativeCall();
                finishCallTransportDiscovery(tr("Could not rotate the media key after a participant left."));
            }
            syncNativeKeys();
        }
        if (nativeConnected) participants.insert(profile);
        QVariantList tiles;
        const auto profiles = roomProfiles.value(callRoomId);
        for (const auto &user : participants) {
            const auto member = profiles.value(user);
            const auto membership = member["membership"].toString();
            if (!membership.isEmpty() && membership != "join") continue;
            const bool local = user == profile;
            const auto name = member["displayname"].toString(local ? accountName : QString());
            const auto encryptedAvatar = member["avatar_file"].toObject()["url"].toString();
            const auto avatar = encryptedAvatar.isEmpty()
                ? avatarUrl(member["avatar_url"].toString(local ? accountAvatar : QString()))
                : avatarUrl(encryptedAvatar, true);
            tiles.append(QVariantMap{{"userId", user}, {"name", name.isEmpty() ? user : name},
                                     {"avatar", avatar}, {"local", local}});
        }
        std::sort(tiles.begin(), tiles.end(), [](const QVariant &a, const QVariant &b) {
            const auto left = a.toMap();
            const auto right = b.toMap();
            if (left["local"].toBool() != right["local"].toBool()) return left["local"].toBool();
            const int compare = left["name"].toString().compare(right["name"].toString(), Qt::CaseInsensitive);
            return compare != 0 ? compare < 0 : left["userId"].toString() < right["userId"].toString();
        });
        if (tiles != callParticipants) {
            callParticipants = tiles;
            emit callParticipantsChanged();
        }
    }

    struct RemoteVideo {
        RemoteVideo(const std::shared_ptr<livekit::Track> &track, QByteArray participant, bool screen)
            : identity(std::move(participant)), sharing(screen) {
            livekit::VideoStream::Options options;
            options.capacity = 2;
            stream = livekit::VideoStream::fromTrack(track, options);
            reader = std::thread([this] {
                try {
                    livekit::VideoFrameEvent event;
                    while (stream->read(event)) {
                        const auto &frame = event.frame;
                        if (frame.type() != livekit::VideoBufferType::RGBA || frame.width() <= 0 || frame.height() <= 0) continue;
                        QImage image(frame.data(), frame.width(), frame.height(), frame.width() * 4, QImage::Format_RGBA8888);
                        auto copy = image.copy();
                        std::lock_guard lock(mutex);
                        latest = std::move(copy);
                    }
                } catch (const std::exception &) {}
            });
        }

        ~RemoteVideo() {
            stream->close();
            reader.join();
        }

        QByteArray identity;
        bool sharing;
        std::shared_ptr<livekit::VideoStream> stream;
        std::thread reader;
        std::mutex mutex;
        QImage latest;
    };

    struct NativeSession : livekit::RoomDelegate {
        livekit::PlatformAudio audio;
        livekit::Room room;
        std::shared_ptr<livekit::LocalAudioTrack> microphone;
        struct Publication {
            QByteArray identity;
            std::shared_ptr<livekit::RemoteTrackPublication> info;
            std::shared_ptr<livekit::Track> video;
        };
        std::mutex publicationMutex;
        QMap<QString, Publication> publications;

        NativeSession() { room.setDelegate(this); }

        void onTrackPublished(livekit::Room &, const livekit::TrackPublishedEvent &event) override {
            if (!event.participant || !event.publication) return;
            std::lock_guard lock(publicationMutex);
            auto &entry = publications[QString::fromStdString(event.publication->sid())];
            entry.identity = QByteArray::fromStdString(event.participant->identity());
            entry.info = event.publication;
        }

        void onTrackUnpublished(livekit::Room &, const livekit::TrackUnpublishedEvent &event) override {
            if (!event.publication) return;
            std::lock_guard lock(publicationMutex);
            publications.remove(QString::fromStdString(event.publication->sid()));
        }

        void onParticipantDisconnected(livekit::Room &, const livekit::ParticipantDisconnectedEvent &event) override {
            if (!event.participant) return;
            const auto identity = QByteArray::fromStdString(event.participant->identity());
            std::lock_guard lock(publicationMutex);
            for (auto it = publications.begin(); it != publications.end();) {
                if (it->identity == identity) it = publications.erase(it);
                else ++it;
            }
        }

        void onTrackSubscribed(livekit::Room &, const livekit::TrackSubscribedEvent &event) override {
            if (!event.track || !event.participant || !event.publication
                || event.track->kind() != livekit::TrackKind::KIND_VIDEO
                || event.publication->encryptionType() != livekit::EncryptionType::GCM) return;
            std::lock_guard lock(publicationMutex);
            publications.insert(QString::fromStdString(event.publication->sid()),
                {QByteArray::fromStdString(event.participant->identity()), event.publication, event.track});
        }

        void onTrackUnsubscribed(livekit::Room &, const livekit::TrackUnsubscribedEvent &event) override {
            if (!event.publication) return;
            std::lock_guard lock(publicationMutex);
            const auto it = publications.find(QString::fromStdString(event.publication->sid()));
            if (it != publications.end()) it->video.reset();
        }

        void loadInitialPublications() {
            for (const auto &weak : room.remoteParticipants()) {
                const auto participant = weak.lock();
                if (!participant) continue;
                for (const auto &entry : participant->trackPublications()) {
                    onTrackPublished(room, {entry.second, participant.get()});
                }
            }
        }

        QMap<QString, Publication> remotePublications() {
            std::lock_guard lock(publicationMutex);
            return publications;
        }

        ~NativeSession() {
            try {
                if (room.connectionState() != livekit::ConnectionState::Disconnected)
                    room.disconnect();
            } catch (const std::exception &) {}
            room.setDelegate(nullptr);
        }

        void setKey(const QByteArray &identity, const MatrixRtcWire::MediaKey &key) {
            const auto encryption = room.e2eeManager().lock();
            const auto provider = encryption ? encryption->keyProvider().lock() : nullptr;
            if (!provider) throw std::runtime_error("LiveKit key provider unavailable");
            provider->setKey(identity.toStdString(), {key.bytes.cbegin(), key.bytes.cend()}, key.index);
            encryption->setEnabled(true);
        }
    };

    struct NativeCapture {
        QVideoSink sink;
        QMediaCaptureSession session;
        std::unique_ptr<QCamera> camera;
        std::unique_ptr<QScreenCapture> screen;
        std::unique_ptr<QWindowCapture> window;
        std::shared_ptr<livekit::VideoSource> source;
        std::shared_ptr<livekit::LocalVideoTrack> track;
        QSize frameSize;
        QImage latest;
        QElapsedTimer frameClock;
    };

    void stopNativeCapture(std::unique_ptr<NativeCapture> &capture) {
        if (!capture) return;
        if (capture->camera) capture->camera->stop();
        if (capture->screen) capture->screen->stop();
        if (capture->window) capture->window->stop();
        if (capture->track && nativeSession) {
            try {
                if (auto participant = nativeSession->room.localParticipant().lock())
                    participant->unpublishTrack(capture->track->sid());
            } catch (const std::exception &) {
                // Disconnect if the server might still be publishing our capture.
                QTimer::singleShot(0, this, [this, member = nativeSelf.memberId] {
                    if (nativeSelf.memberId == member) closeCall();
                });
            }
        }
        capture.reset();
    }

    void deferNativeCaptureFailure(bool share, QMediaCaptureSession *owner) {
        QTimer::singleShot(0, this, [this, share, capture = QPointer<QMediaCaptureSession>(owner)] {
            if (capture) failNativeCapture(share);
        });
    }

    void failNativeCapture(bool share) {
        auto &capture = share ? localShare : localCamera;
        if (!capture) return;
        stopNativeCapture(capture);
        finishCallTransportDiscovery(share ? tr("Could not share the screen.") : tr("Could not open the camera."));
    }

    void pushNativeVideo(bool share, const QVideoFrame &frame) {
        auto &capture = share ? localShare : localCamera;
        if (!capture || !nativeConnected || !frame.isValid()) return;
        if (capture->frameClock.isValid() && capture->frameClock.elapsed() < 32) return;
        capture->frameClock.start();
        auto image = frame.toImage();
        if (image.isNull()) return;
        try {
            if (!capture->source) {
                const auto participant = nativeSession->room.localParticipant().lock();
                if (!participant) return;
                capture->frameSize = image.size();
                if (image.width() > 1920 || image.height() > 1080)
                    capture->frameSize = image.size().scaled(1920, 1080, Qt::KeepAspectRatio);
                capture->source = std::make_shared<livekit::VideoSource>(capture->frameSize.width(), capture->frameSize.height());
                capture->track = participant->publishVideoTrack(share ? "screen" : "camera", capture->source,
                    share ? livekit::TrackSource::SOURCE_SCREENSHARE : livekit::TrackSource::SOURCE_CAMERA);
            }
            if (image.size() != capture->frameSize) image = image.scaled(capture->frameSize);
            image = image.convertToFormat(QImage::Format_RGBA8888);
            const auto *data = image.constBits();
            const livekit::VideoFrame outgoing(image.width(), image.height(), livekit::VideoBufferType::RGBA,
                                                {data, data + image.sizeInBytes()});
            capture->source->captureFrame(outgoing);
            capture->latest = std::move(image);
        } catch (const std::exception &) {
            deferNativeCaptureFailure(share, &capture->session);
        }
    }

    void renderNativeVideo() {
        if (!nativeSession) return;
        QVariantList tiles;
        const auto write = [this, &tiles](const QString &id, const QString &name, const QImage &image, bool sharing) {
            tiles.append(QVariantMap{{"id", id}, {"name", name}, {"sharing", sharing}});
            if (!image.isNull() && nativeSinks.value(id)) nativeSinks.value(id)->setVideoFrame(QVideoFrame(image));
        };
        if (localCamera && localCamera->track) write("local-camera", profileName(), localCamera->latest, false);
        if (localShare && localShare->track) write("local-screen", profileName(), localShare->latest, true);

        for (auto it = nativeRemoteVideos.cbegin(); it != nativeRemoteVideos.cend(); ++it) {
            const auto &video = it.value();
            for (const auto &peer : std::as_const(nativePeers)) {
                if (video->identity != legacyIdentity(peer) && video->identity != MatrixRtcWire::livekitIdentity(peer)) continue;
                QImage image;
                {
                    std::lock_guard lock(video->mutex);
                    image = std::exchange(video->latest, {});
                }
                const auto name = roomProfiles.value(callRoomId).value(peer.userId)["displayname"].toString(peer.userId);
                write(it.key(), name, image, video->sharing);
                break;
            }
        }
        if (nativeVideoTiles != tiles) {
            nativeVideoTiles = tiles;
            emit nativeVideoTilesChanged();
        }
    }

    void refreshNativeMedia() {
        if (!nativeConnected || !nativeSession) return;
        if (nativeSession->room.connectionState() == livekit::ConnectionState::Disconnected) {
            closeNativeCall();
            finishCallTransportDiscovery(tr("The LiveKit connection ended."));
            return;
        }
        syncNativeSubscriptions();
    }

    static QByteArray legacyIdentity(const MatrixRtcWire::Peer &peer) {
        return (peer.userId + ':' + peer.deviceId).toUtf8();
    }

    QJsonObject nativeMembership(const QString &room, const MatrixRtcWire::Peer &self, const QUrl &service = {}) const {
        QJsonObject member{{"id", self.memberId}, {"membership", service.isEmpty() ? "leave" : "join"}};
        QJsonObject content{{"slot_id", "m.call#ROOM"}, {"msc4354_sticky_key", self.memberId}};
        if (!service.isEmpty()) {
            member.insert("device_id", self.deviceId);
            member.insert("user_id", self.userId);
            const auto field = service.scheme() == "wss" ? "url" : "livekit_service_url";
            const QJsonObject transport{{"type", "livekit"}, {field, service.toString()}};
            content.insert("application", QJsonObject{{"type", "m.call"}});
            content.insert("transports", QJsonObject{{"published", QJsonArray{transport}}, {"can_subscribe", QJsonArray{"livekit"}}});
        }
        content.insert("member", member);
        return {{"room_id", room}, {"type", "org.matrix.msc4143.rtc.member"},
                {"sticky_duration_ms", 120000}, {"content", content}};
    }

    void nativeRequest(const QString &command, const QJsonObject &payload, std::function<void(QJsonValue)> handler) {
        const int request = transmit(command, payload);
        nativePending.insert(request, std::move(handler));
        QTimer::singleShot(15000, this, [this, request] {
            if (auto callback = nativePending.take(request)) callback(QJsonValue(QJsonValue::Undefined));
        });
    }

    void tryNextNativeTransport(const QUrl &service, const QString &error) {
        callTransports.removeAll(service);
        if (callTransports.isEmpty()) finishCallTransportDiscovery(error);
        else authenticateNativeCall();
    }

    void authenticateNativeCall() {
        if (callTransports.isEmpty()) return;
        if (nativeJoinWatcher) {
            finishCallTransportDiscovery(tr("The previous call is still closing. Close this panel and try again."));
            return;
        }
        const auto member = nativeSelf.memberId;
        const auto service = callTransports.first();
        finishCallTransportDiscovery(tr("Requesting MatrixRTC credentials…"));
        if (service.scheme() == "wss") {
            const auto payload = MatrixRtcWire::tokenRequest(service, callRoomId, "m.call#ROOM", nativeSelf);
            if (!payload) return;
            nativeRequest("rtc_livekit_get_token", *payload, [this, member, service](const QJsonValue &result) {
                if (nativeSelf.memberId != member) return;
                const auto jwt = result.toObject()["jwt"].toString();
                if (jwt.isEmpty()) {
                    tryNextNativeTransport(service, tr("Could not obtain a LiveKit token from the homeserver."));
                    return;
                }
                connectNativeCall(service, jwt, service);
            });
            return;
        }
        nativeRequest("request_openid_token", {}, [this, member, service](const QJsonValue &result) {
            if (nativeSelf.memberId != member) return;
            if (result.toObject()["access_token"].toString().isEmpty()) {
                tryNextNativeTransport(service, tr("Could not obtain Matrix OpenID credentials."));
                return;
            }

            auto endpoint = service;
            auto path = endpoint.path();
            if (path.endsWith('/')) path.chop(1);
            endpoint.setPath(path + "/sfu/get");
            QNetworkRequest request(endpoint);
            request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::ManualRedirectPolicy);
            request.setAttribute(QNetworkRequest::CookieLoadControlAttribute, QNetworkRequest::Manual);
            request.setAttribute(QNetworkRequest::CookieSaveControlAttribute, QNetworkRequest::Manual);
            request.setHeader(QNetworkRequest::ContentTypeHeader, "application/json");
            request.setTransferTimeout(15000);
            const QJsonObject payload{{"room", callRoomId}, {"openid_token", result}, {"device_id", nativeSelf.deviceId}};
            auto *reply = http.post(request, QJsonDocument(payload).toJson(QJsonDocument::Compact));
            connect(reply, &QNetworkReply::finished, this, [this, reply, member, service] {
                reply->deleteLater();
                if (nativeSelf.memberId != member) return;
                const auto status = reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
                const auto result = QJsonDocument::fromJson(reply->readAll()).object();
                const QUrl url(result["url"].toString(), QUrl::StrictMode);
                const auto jwt = result["jwt"].toString();
                if (reply->error() != QNetworkReply::NoError || status < 200 || status >= 300
                    || jwt.isEmpty() || !url.isValid() || url.scheme() != "wss" || url.host().isEmpty()
                    || !url.userInfo().isEmpty() || url.hasFragment()) {
                    tryNextNativeTransport(service, tr("The MatrixRTC service did not return valid LiveKit credentials."));
                    return;
                }
                connectNativeCall(url, jwt, service);
            });
        });
    }

    static MatrixRtcWire::MediaKey randomNativeKey() {
        QByteArray bytes;
        for (int i = 0; i < 8; ++i) {
            const auto random = QRandomGenerator::system()->generate();
            bytes.append(reinterpret_cast<const char *>(&random), sizeof(random));
        }
        return {bytes, 0};
    }

    bool rotateNativeKey() {
        if (!nativeSession) return false;
        try {
            nativeKey = randomNativeKey();
            const auto identity = nativeService.scheme() == "wss" ? MatrixRtcWire::livekitIdentity(nativeSelf) : legacyIdentity(nativeSelf);
            nativeSession->setKey(identity, nativeKey);
            ++nativeKeyGeneration;
            nativeKeyRecipients.clear();
            nativeKeyRequests.clear();
            return true;
        } catch (const std::exception &) {
            return false;
        }
    }

    void connectNativeCall(const QUrl &url, const QString &jwt, const QUrl &service) {
        nativeKey = randomNativeKey();
        using Result = std::shared_ptr<NativeSession>;
        auto *watcher = new QFutureWatcher<Result>(this);
        nativeJoinWatcher = watcher;
        const auto self = nativeSelf;
        finishCallTransportDiscovery(tr("Connecting to LiveKit…"));
        connect(watcher, &QFutureWatcher<Result>::finished, this, [this, watcher, self, service] {
            const auto session = watcher->result();
            watcher->deleteLater();
            nativeJoin = {};
            if (nativeSelf.memberId != self.memberId) return;
            nativeSession = session;
            if (!nativeSession) {
                finishCallTransportDiscovery(tr("Could not connect to the encrypted LiveKit room."));
                return;
            }
            nativeService = service;
            publishNativeMembership();
        });
        const auto identity = service.scheme() == "wss" ? MatrixRtcWire::livekitIdentity(self) : legacyIdentity(self);
        nativeJoin = QtConcurrent::run([url, jwt, identity, key = nativeKey]() -> Result {
            try {
                auto session = std::make_shared<NativeSession>();
                livekit::RoomOptions options;
                options.connect_timeout = std::chrono::seconds(10);
                options.join_retries = 0;
                options.auto_subscribe = false;
                auto &keys = options.encryption.emplace().key_provider_options;
                keys.key_derivation_function = livekit::KeyDerivationFunction::HKDF;
                keys.ratchet_window_size = 10;
                keys.key_ring_size = 256;
                if (!session->room.connect(url.toEncoded().toStdString(), jwt.toStdString(), options)) return {};
                const auto participant = session->room.localParticipant().lock();
                if (!participant || participant->identity() != identity.toStdString()) return {};
                session->setKey(identity, key);
                session->loadInitialPublications();
                return session;
            } catch (const std::exception &) {
                return {};
            }
        });
        watcher->setFuture(nativeJoin);
    }

    QJsonObject nativeStateMembership(const QString &room, const MatrixRtcWire::Peer &self,
                                        const QUrl &service = {}) const {
        auto key = self.userId + '_' + self.deviceId + "_m.call";
        const auto version = rooms.value(room)["creation_content"].toObject()["room_version"].toString();
        if (!QRegularExpression(R"(^org\.matrix\.msc(3757|3779)\b)").match(version).hasMatch()) key.prepend('_');
        QJsonObject content;
        if (!service.isEmpty()) {
            const auto field = service.scheme() == "wss" ? "url" : "livekit_service_url";
            const QJsonObject transport{{"type", "livekit"}, {field, service.toString()}};
            content = {{"application", "m.call"}, {"call_id", ""}, {"scope", "m.room"},
                       {"device_id", self.deviceId}, {"membershipID", self.memberId},
                       {"created_ts", nativeStateCreated},
                       {"expires", 120000 + QDateTime::currentMSecsSinceEpoch() - nativeStateCreated},
                       {"focus_active", QJsonObject{{"type", "livekit"}, {"focus_selection", "multi_sfu"}}},
                       {"foci_preferred", QJsonArray{transport}}};
        }
        return {{"room_id", room}, {"type", "org.matrix.msc3401.call.member"}, {"state_key", key}, {"content", content}};
    }

    void leaveNativeMembership(const QString &room, const MatrixRtcWire::Peer &self, bool stateEvent) {
        if (!ready) return;
        if (stateEvent) {
            // A newer call in the same room will replace the state event.
            if (room == callRoomId && !nativeSelf.memberId.isEmpty() && nativeSelf.memberId != self.memberId) return;
            transmit("set_state", nativeStateMembership(room, self));
        } else {
            transmit("send_sticky_event", nativeMembership(room, self));
        }
    }

    void publishNativeMembership() {
        const auto room = callRoomId;
        const auto self = nativeSelf;
        const bool stateEvent = nativeStateEvent;
        const auto membership = stateEvent ? nativeStateMembership(room, self, nativeService)
                                           : nativeMembership(room, self, nativeService);
        nativeRequest(stateEvent ? "set_state" : "send_sticky_event", membership,
                      [this, room, self, stateEvent](const QJsonValue &result) {
            if (nativeSelf.memberId != self.memberId) {
                if (!result.toString().isEmpty()) leaveNativeMembership(room, self, stateEvent);
                return;
            }
            if (result.toString().isEmpty()) {
                if (!nativeConnected && !stateEvent && result.isNull()) {
                    nativeStateEvent = true;
                    nativeStateCreated = QDateTime::currentMSecsSinceEpoch();
                    publishNativeMembership();
                    return;
                }
                leaveNativeMembership(room, self, stateEvent);
                closeNativeCall();
                finishCallTransportDiscovery(tr("The homeserver rejected the MatrixRTC membership."));
                return;
            }
            nativeConnected = true;
            nativeVideoTimer.start();
            nativeMediaTimer.start();
            syncNativeKeys();
            refreshCallParticipants();
            finishCallTransportDiscovery(tr("Connected to the encrypted call."));
            QTimer::singleShot(45000, this, [this, self] {
                if (nativeConnected && nativeSelf.memberId == self.memberId) publishNativeMembership();
            });
        });
    }

    void receiveNativeKeys(const QJsonArray &events) {
        for (const auto &value : events) {
            const auto event = value.toObject();
            if (event["type"].toString() != MatrixRtcWire::KeyEventType || !event["encrypted"].toBool()
                || event["content"].toObject()["room_id"].toString() != callRoomId) continue;
            if (nativeKeyInbox.size() >= 128) nativeKeyInbox.removeFirst();
            nativeKeyInbox.append({QDateTime::currentMSecsSinceEpoch(), event});
        }
        if (!events.isEmpty()) {
            syncNativeKeys();
            refreshCallParticipants();
        }
    }

    void syncNativeSubscriptions() {
        if (!nativeConnected || !nativeSession) return;
        QSet<QByteArray> activeIdentities;
        for (const auto &peer : std::as_const(nativePeers)) {
            activeIdentities.insert(legacyIdentity(peer));
            activeIdentities.insert(MatrixRtcWire::livekitIdentity(peer));
        }
        try {
            const auto publications = nativeSession->remotePublications();
            for (auto it = nativeRemoteVideos.begin(); it != nativeRemoteVideos.end();) {
                const auto publication = publications.constFind(it.key());
                if (publication == publications.cend() || !publication->video || !activeIdentities.contains(publication->identity))
                    it = nativeRemoteVideos.erase(it);
                else
                    ++it;
            }
            for (auto it = publications.cbegin(); it != publications.cend(); ++it) {
                const auto &track = it->info;
                const bool desired = activeIdentities.contains(it->identity)
                    && (track->kind() == livekit::TrackKind::KIND_VIDEO
                        || (track->kind() == livekit::TrackKind::KIND_AUDIO && !nativeOutputMuted))
                    && track->encryptionType() == livekit::EncryptionType::GCM;
                if (track->subscribed() != desired) track->setSubscribed(desired);
                if (desired && it->video && !nativeRemoteVideos.contains(it.key())) {
                    const bool sharing = track->source() == livekit::TrackSource::SOURCE_SCREENSHARE;
                    nativeRemoteVideos.insert(it.key(), std::make_shared<RemoteVideo>(it->video, it->identity, sharing));
                }
            }
        } catch (const std::exception &) {
            finishCallTransportDiscovery(tr("Could not update incoming call audio."));
        }
    }

    void syncNativeKeys() {
        if (!nativeConnected || !nativeSession) return;
        const auto now = QDateTime::currentMSecsSinceEpoch();
        for (auto it = nativeKeyInbox.begin(); it != nativeKeyInbox.end();) {
            bool consumed = now - it->first > 20000;
            for (const auto &peer : std::as_const(nativePeers)) {
                if (consumed) break;
                if (auto key = MatrixRtcWire::receivedKey(it->second, callRoomId, peer)) {
                    try {
                        nativeSession->setKey(legacyIdentity(peer), *key);
                        nativeSession->setKey(MatrixRtcWire::livekitIdentity(peer), *key);
                    } catch (const std::exception &) {
                        finishCallTransportDiscovery(tr("Could not install a participant's media key."));
                    }
                    consumed = true;
                }
            }
            if (consumed) it = nativeKeyInbox.erase(it);
            else ++it;
        }
        for (const auto &peer : std::as_const(nativePeers)) {
            const auto id = MatrixRtcWire::livekitIdentity(peer);
            if (nativeKeyRecipients.contains(id) || nativeKeyRequests.contains(id)) continue;
            const auto request = MatrixRtcWire::encryptedKeyRequest(callRoomId, nativeSelf, peer, nativeKey);
            if (!request) continue;
            const auto self = nativeSelf.memberId;
            nativeKeyRequests.insert(id);
            nativeRequest("send_to_device", *request, [this, self, id, generation = nativeKeyGeneration](const QJsonValue &result) {
                if (nativeSelf.memberId != self || nativeKeyGeneration != generation) return;
                nativeKeyRequests.remove(id);
                if (result.isObject()) nativeKeyRecipients.insert(id);
            });
        }
        syncNativeSubscriptions();
    }

    void closeNativeCall() {
        if (nativeConnected) leaveNativeMembership(callRoomId, nativeSelf, nativeStateEvent);
        nativeStateEvent = false;
        nativeVideoTimer.stop();
        nativeMediaTimer.stop();
        stopNativeCapture(localCamera);
        stopNativeCapture(localShare);
        nativeRemoteVideos.clear();
        nativeVideoTiles.clear();
        nativeSinks.clear();
        emit nativeVideoTilesChanged();
        nativeConnected = false;
        nativeMuted = true;
        nativeOutputMuted = false;
        nativeSelf = {};
        nativeSession.reset();
        nativeService.clear();
        nativePeers.clear();
        nativeKeyInbox.clear();
        nativeKeyRecipients.clear();
        nativeKeyRequests.clear();
        nativeKey = {};
    }

    QHash<int, std::function<void(QJsonValue)>> nativePending;
    QFuture<std::shared_ptr<NativeSession>> nativeJoin;
    QPointer<QFutureWatcher<std::shared_ptr<NativeSession>>> nativeJoinWatcher;
    bool nativeOutputMuted = false;
    bool nativeStateEvent = false;
    qint64 nativeStateCreated = 0;
    std::shared_ptr<NativeSession> nativeSession;
    MatrixRtcWire::Peer nativeSelf;
    QList<MatrixRtcWire::Peer> nativePeers;
    MatrixRtcWire::MediaKey nativeKey{};
    QList<QPair<qint64, QJsonObject>> nativeKeyInbox;
    QSet<QByteArray> nativeKeyRecipients, nativeKeyRequests;
    quint64 nativeKeyGeneration = 0;
    QUrl nativeService;
    QTimer nativeVideoTimer, nativeMediaTimer;
    std::unique_ptr<NativeCapture> localCamera, localShare;
    QList<QPointer<QScreen>> shareScreens;
    QList<QCapturableWindow> shareWindows;
    QHash<QString, QPointer<QVideoSink>> nativeSinks;
    QMap<QString, std::shared_ptr<RemoteVideo>> nativeRemoteVideos;

    QVariantList nativeDevices(bool input) const {
        QVariantList result;
        if (nativeSession) {
            try {
                const auto devices = input ? nativeSession->audio.recordingDevices() : nativeSession->audio.playoutDevices();
                for (const auto &device : devices) {
                    result.append(QVariantMap{{"name", QString::fromStdString(device.name)},
                                              {"id", QString::fromStdString(device.id)}});
                }
            } catch (const std::exception &) {}
        }
        return result;
    }

    QVariantList nativeVideoTiles;
    bool nativeConnected = false;
    bool nativeMuted = true;

    void discoverCallTransports() {
        callTransports.clear();
        const int request = callTransportRequest = transmit("get_rtc_transports", {});
        callTransportStatus = tr("Checking the homeserver’s MatrixRTC transport…");
        emit callControlsChanged();
        QTimer::singleShot(8000, this, [this, request] {
            if (request == callTransportRequest) {
                callTransportRequest = 0;
                resolveLegacyCallTransport();
            }
        });
    }

    void finishCallTransportDiscovery(const QString &status) {
        callTransportRequest = 0;
        callTransportStatus = status;
        emit callControlsChanged();
    }

    void readCallTransports(const QJsonArray &transports) {
        for (const auto &value : transports) {
            const auto transport = value.toObject();
            if (transport["type"].toString() != "livekit") continue;
            const QUrl websocket(transport["url"].toString(), QUrl::StrictMode);
            if (MatrixRtcWire::tokenRequest(websocket, callRoomId, "m.call#ROOM", nativeSelf)
                && !callTransports.contains(websocket))
                callTransports.append(websocket);

            const QUrl service(transport["livekit_service_url"].toString(), QUrl::StrictMode);
            if (service.isValid() && service.scheme() == "https" && !service.host().isEmpty()
                && service.userInfo().isEmpty() && !service.hasQuery() && !service.hasFragment()
                && !callTransports.contains(service))
                callTransports.append(service);
        }
    }

    void resolveLegacyCallTransport() {
        const auto colon = profile.indexOf(':');
        if (colon < 0) {
            finishCallTransportDiscovery(tr("The homeserver did not advertise a LiveKit service."));
            return;
        }
        const auto authority = profile.mid(colon + 1);
        const QUrl url("https://" + authority + "/.well-known/matrix/client", QUrl::StrictMode);
        if (!url.isValid() || url.authority() != authority || url.host().isEmpty()
            || !url.userInfo().isEmpty() || url.hasQuery() || url.hasFragment()) {
            finishCallTransportDiscovery(tr("The homeserver did not advertise a LiveKit service."));
            return;
        }
        const auto member = nativeSelf.memberId;
        QNetworkRequest request(url);
        request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::NoLessSafeRedirectPolicy);
        request.setAttribute(QNetworkRequest::CookieLoadControlAttribute, QNetworkRequest::Manual);
        request.setAttribute(QNetworkRequest::CookieSaveControlAttribute, QNetworkRequest::Manual);
        request.setTransferTimeout(8000);
        auto *reply = http.get(request);
        connect(reply, &QNetworkReply::finished, this, [this, reply, member] {
            reply->deleteLater();
            if (nativeSelf.memberId != member) return;
            if (reply->error() == QNetworkReply::NoError) {
                const auto object = QJsonDocument::fromJson(reply->readAll()).object();
                if (reply->url().scheme() == "https")
                    readCallTransports(object["org.matrix.msc4143.rtc_foci"].toArray());
            }
            if (!callTransports.isEmpty()) authenticateNativeCall();
            else finishCallTransportDiscovery(tr("The homeserver did not advertise a LiveKit service."));
        });
    }

    static QJsonObject messageParams(const QString &room, const QString &text, const QString &replyTo) {
        QJsonObject params{{"room_id", room}, {"text", text}};
        if (!replyTo.isEmpty()) {
            const QJsonObject target{{"event_id", replyTo}};
            params.insert("relates_to", QJsonObject{{"m.in_reply_to", target}});
        }
        return params;
    }

    void cancelImageUpload() {
        imageUploadRoom.clear();
        imageSendRequest = 0;
        if (auto *reply = std::exchange(imageUpload, nullptr))
            reply->abort();
    }

    QUrl avatarUrl(const QString &mxc, bool encrypted = false) const {
        return mediaUrl(mxc, encrypted, true);
    }

    QUrl messageMediaUrl(const QJsonObject &content, const QString &plainKey, const QString &encryptedKey) const {
        const auto encrypted = content[encryptedKey].toObject();
        if (!encrypted.isEmpty())
            return mediaUrl(encrypted["url"].toString(), true);
        return mediaUrl(content[plainKey].toString(), false);
    }

    QUrl mediaUrl(const QString &mxc, bool encrypted, bool avatar = false) const {
        if (base.isEmpty() || imageToken.isEmpty()) return {};
        static const QRegularExpression pattern(
            R"(^mxc://([A-Za-z0-9.\-:\[\]]+)/([A-Za-z0-9_\-]+)\z)");
        const auto match = pattern.match(mxc);
        if (!match.hasMatch()) return {};

        QUrl url = base;
        url.setPath("/_gomuks/media/" + match.captured(1) + "/" + match.captured(2));
        QUrlQuery query;
        if (avatar) query.addQueryItem("thumbnail", "avatar");
        query.addQueryItem("encrypted", encrypted ? "true" : "false");
        query.addQueryItem("image_auth", imageToken);
        url.setQuery(query);
        return url;
    }

    void loadRoomProfiles() {
        if (!ready || selected.isEmpty() || loadedRoomProfiles.contains(selected)) return;
        const auto room = selected;
        rpc("get_room_state", {{"room_id", room}, {"include_members", true}}, [this, room](QJsonValue reply) {
            if (!rooms.contains(room)) return;
            loadedRoomProfiles.insert(room);
            auto &profiles = roomProfiles[room];
            for (const auto &entry : reply.toArray()) {
                const auto event = entry.toObject();
                const auto user = event["state_key"].toString();
                if (event["type"].toString() == "m.room.member" && !user.isEmpty() && !profiles.contains(user))
                    profiles.insert(user, event["content"].toObject());
            }
            if (room == selected) emit timelineChanged();
        });
    }

    void setStatus(QString message) { status = std::move(message); emit changed(); }
    void readStartup() {
        stdoutBuffer += process.readAllStandardOutput();
        while (stdoutBuffer.contains('\n')) {
            const auto end = stdoutBuffer.indexOf('\n');
            const auto line = stdoutBuffer.left(end);
            stdoutBuffer.remove(0, end+1);
            const auto frame = QJsonDocument::fromJson(line).object();
            if (!frame["started"].toBool() || !base.isEmpty()) continue;
            const QUrl candidate("http://" + frame["address"].toString());
            if (!QHostAddress(candidate.host()).isLoopback() || candidate.port() <= 0 ||
                !candidate.path().isEmpty() || !candidate.userInfo().isEmpty() ||
                candidate.hasQuery() || candidate.hasFragment()) {
                setStatus("The backend returned an invalid loopback address.");
                process.terminate();
                return;
            }
            base = candidate;
            authenticate();
        }
        if (stdoutBuffer.size() > 1024 * 1024) stdoutBuffer.clear();
    }
    QByteArray desktopAuthorization() const {
        return "Basic " + (QByteArray("desktop-key:") + key).toBase64();
    }

    void authenticate() {
        QUrl url = base;
        url.setPath("/_gomuks/auth");
        url.setQuery("insecure_cookie=true");
        QNetworkRequest request(url);
        request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::ManualRedirectPolicy);
        request.setTransferTimeout(5000);
        request.setRawHeader("Authorization", desktopAuthorization());
        request.setHeader(QNetworkRequest::ContentTypeHeader, "application/x-www-form-urlencoded");
        auto *reply = http.post(request, QByteArray());
        connect(reply, &QNetworkReply::finished, this, [this, reply] {
            const int code = reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
            reply->deleteLater();
            QByteArray cookie;
            QUrl cookieUrl = base;
            cookieUrl.setPath("/_gomuks/websocket");
            for (const auto &entry : http.cookieJar()->cookiesForUrl(cookieUrl))
                if (entry.name() == "gomuks_auth") cookie = entry.toRawForm(QNetworkCookie::NameAndValueOnly);
            if (code < 200 || code >= 300 || cookie.isEmpty()) {
                startup.stop();
                setStatus("Local backend authentication failed.");
                return;
            }
            QUrl url = base;
            url.setScheme("ws");
            url.setPath("/_gomuks/websocket");
            QNetworkRequest request(url);
            request.setRawHeader("Cookie", cookie);
            lastEvent = 0;
            socket.open(request);
        });
    }
    int transmit(const QString &command, QJsonObject data) {
        return transmitValue(command, data);
    }
    int transmitValue(const QString &command, QJsonValue data) {
        const int id = ++sequence;
        socket.sendTextMessage(QString::fromUtf8(QJsonDocument(QJsonObject{
            {"command", command}, {"request_id", id}, {"data", data}}).toJson(QJsonDocument::Compact)));
        return id;
    }
    void rpc(const QString &command, QJsonObject data, std::function<void(QJsonValue)> handler = {}) {
        if (ready && !loggingOut()) pending.insert(transmit(command, std::move(data)), std::move(handler));
    }
    void insert(const QString &room, const QJsonObject &event) {
        const auto type = event["type"].toString();
        if (room.isEmpty() || (type != "m.room.message" && type != "m.room.encrypted" && type != "m.sticker")
            || event["relation_type"].toString() == "m.replace") return;
        auto id = event["rowid"].toInteger();
        if (id != 0) history[room][id] = event;
    }
    void selectionChanged() {
        const auto meta = rooms.value(selected);
        roomName = meta["name"].toString(selected);
        encrypted = meta["encryption_event"].isObject();
        emit changed();
        emit timelineChanged();
    }
    void clearRoomData() {
        rooms.clear();
        history.clear();
        roomProfiles.clear();
        loadedRoomProfiles.clear();
        spaces = {};
    }

    void receive(const QString &message) {
        const auto frame = QJsonDocument::fromJson(message.toUtf8()).object();
        const auto name = frame["command"].toString();
        const auto data = frame["data"].toObject();
        auto id = frame["request_id"].toInteger();
        if (id < 0) lastEvent = id;
        if (name == "pong") { misses = 0; return; }
        if (name == "response" || name == "error") {
            if (loggingOut() && id == logoutRequest) {
                if (name == "error") {
                    logoutRequest = 0;
                    setStatus("Could not sign out. Restart untethered and check your connection before trying again.");
                }
                // A successful logout sends a signed-out client state.
                return;
            }
            if (imageSendRequest != 0 && id == imageSendRequest) {
                const auto room = imageUploadRoom;
                cancelImageUpload();
                const auto event = frame["data"].toObject();
                if (name == "error" || event["event_id"].toString().isEmpty()) {
                    setStatus("Could not send the image. You can try again.");
                } else {
                    insert(room, event);
                    emit imageSent(room);
                    setStatus("");
                    if (room == selected) emit timelineChanged();
                }
                return;
            }
            if (callMemberRequest != 0 && id == callMemberRequest) {
                callMemberRequest = 0;
                const auto events = frame["data"].toArray();
                if (waitingForCallState) {
                    if (name == "response" || !callStickyMembers.isEmpty())
                        readCallParticipants(events);
                } else {
                    callStickyMembers = events;
                    waitingForCallState = true;
                    requestCallMembers("get_room_state");
                }
                return;
            }
            if (callTransportRequest != 0 && id == callTransportRequest) {
                callTransportRequest = 0;
                if (name == "response") readCallTransports(data["rtc_transports"].toArray());
                if (!callTransports.isEmpty()) authenticateNativeCall();
                else resolveLegacyCallTransport();
                return;
            }
            if (auto callback = nativePending.take(int(id))) {
                callback(name == "response" ? frame["data"] : QJsonValue{});
                return;
            }
            if (!pending.contains(int(id))) return;
            auto callback = pending.take(int(id));
            if (name == "error") setStatus("Matrix request failed. Check your homeserver, credentials or room access.");
            else if (callback) callback(frame["data"]);
        } else if (name == "image_auth_token") {
            imageToken = frame["data"].toString();
            emit changed();
            emit timelineChanged();
        } else if (name == "client_state") {
            loggedIn = data["is_logged_in"].toBool();
            verified = data["is_verified"].toBool();
            profile = data["user_id"].toString();
            deviceId = data["device_id"].toString();
            accountName = data["displayname"].toString();
            accountAvatar = data["avatar_url"].toString();
            if (!loggedIn) {
                closeCall();
                logoutRequest = 0;
                pending.clear();
                cancelImageUpload();
                clearRoomData();
                selected.clear();
                status = "Sign in to Matrix";
                ready = true;
                selectionChanged();
            } else {
                if (ready && !loggingOut()) setStatus("");
                else emit changed();
                emit timelineChanged();
            }
        } else if (name == "init_complete") {
            ready = true;
            setStatus(loggedIn ? "" : "Sign in to Matrix");
            loadRoomProfiles();
            transmitValue("listen_to_device", true);
        } else if (name == "sync_complete") {
            if (!loggedIn) return;
            if (nativeCall()) receiveNativeKeys(data["to_device"].toArray());
            if (data["clear_state"].toBool()) clearRoomData();
            const auto roomUpdates = data["rooms"].toObject();
            for (auto it = roomUpdates.begin(); it != roomUpdates.end(); ++it) {
                const auto delta = it.value().toObject();
                if (delta["meta"].isObject()) rooms[it.key()] = delta["meta"].toObject();
                if (delta["reset"].toBool()) history.remove(it.key());
                QSet<qint64> timeline;
                for (const auto &row : delta["timeline"].toArray()) timeline.insert(row.toObject()["event_rowid"].toInteger());
                const auto members = delta["state"].toObject()["m.room.member"].toObject();
                for (const auto &entry : delta["events"].toArray()) {
                    const auto event = entry.toObject();
                    const auto row = event["rowid"].toInteger();
                    if (timeline.contains(row)) insert(it.key(), event);
                    const auto user = event["state_key"].toString();
                    if (event["type"].toString() == "m.room.member" && members.value(user).toInteger(-1) == row)
                        roomProfiles[it.key()].insert(user, event["content"].toObject());
                }
            }
            const auto edges = data["space_edges"].toObject();
            for (auto it = edges.begin(); it != edges.end(); ++it) spaces.insert(it.key(), it.value());
            for (const auto &room : data["left_rooms"].toArray()) {
                if (callRoomId == room.toString()) closeCall();
                rooms.remove(room.toString());
                history.remove(room.toString());
                roomProfiles.remove(room.toString());
                loadedRoomProfiles.remove(room.toString());
                if (selected == room.toString()) selected.clear();
            }
            selectionChanged();
        } else if (name == "events_decrypted" || name == "send_complete") {
            if (!loggedIn) return;
            auto events = data["events"].toArray();
            if (name == "send_complete") {
                events.append(data["event"]);
                if (!data["error"].isNull() && !data["error"].isUndefined()) setStatus("The message could not be sent.");
            }
            for (const auto &entry : events) {
                const auto event = entry.toObject();
                insert(event["room_id"].toString(data["room_id"].toString()), event);
            }
            emit timelineChanged();
        }
    }

    QProcess process;
    QNetworkAccessManager http;
    QNetworkReply *imageUpload = nullptr;
    QString imageUploadRoom;
    int imageSendRequest = 0;
    QWebSocket socket;
    QString callRoomName, callRoomId, callTransportStatus;
    QTimer callMembersTimer;
    int callMemberRequest = 0;
    bool waitingForCallState = false;
    QJsonArray callStickyMembers;
    QVariantList callParticipants;
    QList<QUrl> callTransports;
    int callTransportRequest = 0;
    QTimer startup, ping;
    QUrl base;
    QByteArray key, stdoutBuffer;
    QHash<int, std::function<void(QJsonValue)>> pending;
    QMap<QString, QJsonObject> rooms;
    QHash<QString, QMap<qint64, QJsonObject>> history;
    QJsonObject spaces;
    QHash<QString, QHash<QString, QJsonObject>> roomProfiles;
    QSet<QString> loadedRoomProfiles;
    QString accountName, accountAvatar, imageToken;
    bool ready = false, loggedIn = false, verified = false, encrypted = false;
    int sequence = 0, misses = 0, logoutRequest = 0;
    qint64 lastEvent = 0;
    QString status = "Starting local gomuks…", profile, deviceId, selected, roomName;
};

static bool migrateLegacyProfile(const QString &root) {
    if (QFileInfo::exists(root)) return true;

    QDir parent = QFileInfo(root).dir();
    const QString previous = parent.filePath("unteathered");
    const QString legacy = QFileInfo(previous).isDir() ? previous : parent.filePath("gomuks-native");
    if (!QFileInfo(legacy).isDir()) return true;
    QLockFile legacyLock(legacy + "/backend.lock");
    legacyLock.setStaleLockTime(0);
    if (!legacyLock.tryLock()) {
        if (QFileInfo::exists(root)) return true;
        qWarning("Close the previous installation before migrating the Matrix profile.");
        return false;
    }
    if (QFileInfo::exists(root)) return true;
    if (!parent.rename(QFileInfo(legacy).fileName(), QFileInfo(root).fileName())) {
        qWarning("Could not move the Matrix profile to its new directory.");
        return false;
    }

    // The directory move also moved the migration lock.
    if (!QFile::remove(root + "/backend.lock")) {
        qWarning("Could not release the Matrix profile migration lock.");
        return false;
    }
    return true;
}

int main(int argc, char **argv) {
    QGuiApplication app(argc, argv);
    app.setApplicationName("untethered");
    app.setApplicationDisplayName("untethered");
    app.setDesktopFileName("untethered");
    QCommandLineParser parser;
    parser.addHelpOption();
    parser.addOption({"backend", "Absolute path to the gomuks binary for development.", "path"});
    parser.addOption({"smoke-test", "Exit successfully after the backend handshake."});
    parser.process(app);
    const QString root = QStandardPaths::writableLocation(QStandardPaths::AppLocalDataLocation);
    if (!migrateLegacyProfile(root) || !QDir().mkpath(root)
        || !QFile::setPermissions(root, QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner))
        return 1;
    QLockFile lock(root + "/backend.lock");
    lock.setStaleLockTime(0);
    if (!lock.tryLock()) { qWarning("untethered is already using this profile."); return 1; }
    QString backend = parser.value("backend");
    if (backend.isEmpty()) backend = QCoreApplication::applicationDirPath() + "/../lib/untethered/gomuks";
    if (!QFileInfo(backend).isExecutable()) backend.clear();
    Chat chat(backend, root);
    MediaNetworkFactory mediaNetwork;
    QQmlApplicationEngine engine;
    engine.setNetworkAccessManagerFactory(&mediaNetwork);
    engine.rootContext()->setContextProperty("chat", &chat);
    if (parser.isSet("smoke-test")) {
        QObject::connect(&chat, &Chat::changed, &app, [&] {
            if (chat.property("ready").toBool()) {
                fprintf(stdout, "Local gomuks authenticated; initial state received.\n");
                app.exit(0);
            }
        });
        QTimer::singleShot(18000, &app, [&] {
            fprintf(stderr, "%s\n", qPrintable(chat.property("status").toString()));
            app.exit(2);
        });
    }
    engine.load(QUrl("qrc:/Main.qml"));
    if (engine.rootObjects().isEmpty()) return 1;
    return app.exec();
}

#include "main.moc"
