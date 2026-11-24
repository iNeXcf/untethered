#include <QtCore>
#include <QGuiApplication>
#include <QClipboard>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlNetworkAccessManagerFactory>
#include <QtNetwork>
#include <QWebSocket>
#include <functional>
#include <algorithm>

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
        for (int i = 0; i < 8; ++i)
            key += QByteArray::number(QRandomGenerator::system()->generate(), 16).rightJustified(8, '0');
        http.setProxy(QNetworkProxy::NoProxy);
        socket.setProxy(QNetworkProxy::NoProxy);
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
            pending.clear();
            logoutRequest = 0;
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
        socket.blockSignals(true);
        process.blockSignals(true);
        socket.abort();
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
    Q_INVOKABLE void copyText(const QString &text) {
        QGuiApplication::clipboard()->setText(text);
    }
    Q_INVOKABLE bool send(QString text, QString replyTo) {
        if (!ready || loggingOut() || selected.isEmpty() || text.trimmed().isEmpty()) return false;
        const auto room = selected;
        QJsonObject params{{"room_id", room}, {"text", text}};
        if (!replyTo.isEmpty()) {
            if (!replyTo.startsWith('$')) return false;
            const QJsonObject target{{"event_id", replyTo}};
            params.insert("relates_to", QJsonObject{{"m.in_reply_to", target}});
        }
        rpc("send_message", params, [this, room](QJsonValue reply) {
            insert(room, reply.toObject());
            if (room == selected) emit timelineChanged();
        });
        return true;
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
            if (hasImage) {
                image = messageMediaUrl(body, "url", "file");
                thumbnail = messageMediaUrl(body["info"].toObject(), "thumbnail_url", "thumbnail_file");
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

private:
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
    void authenticate() {
        QUrl url = base;
        url.setPath("/_gomuks/auth");
        url.setQuery("insecure_cookie=true");
        QNetworkRequest request(url);
        request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::ManualRedirectPolicy);
        request.setTransferTimeout(5000);
        request.setRawHeader("Authorization", "Basic " + (QByteArray("desktop-key:") + key).toBase64());
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
                logoutRequest = 0;
                pending.clear();
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
        } else if (name == "sync_complete") {
            if (!loggedIn) return;
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
    QWebSocket socket;
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
